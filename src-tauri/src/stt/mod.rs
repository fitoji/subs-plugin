//! Speech-to-text sidecar orchestration.
//!
//! Manages the lifecycle of the Whisper MLX Python sidecar process:
//! spawning, stdin JSON Lines protocol, crash recovery with
//! exponential backoff, and state tracking.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
#[cfg(not(debug_assertions))]
use tauri::Manager;
use tauri::{self, AppHandle};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

use crate::events;

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum SidecarState {
    #[serde(rename = "init")]
    Init,
    #[serde(rename = "loading")]
    Loading,
    #[serde(rename = "ready")]
    Ready,
    #[serde(rename = "processing")]
    Processing,
    #[serde(rename = "idle")]
    Idle,
    #[serde(rename = "crashed")]
    Crashed { attempt: u32, backoff_s: u32 },
    #[serde(rename = "fatal")]
    Fatal,
    #[serde(rename = "shutdown")]
    Shutdown,
}

// ---------------------------------------------------------------------------
// Protocol types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum SidecarMessage {
    #[serde(rename = "transcription")]
    Transcription {
        text: String,
        #[serde(rename = "is_final")]
        is_final: bool,
        timestamp: u64,
    },
    #[serde(rename = "status")]
    Status {
        state: String,
        message: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum SttError {
    #[error("Sidecar exited with code {code}")]
    Exit { code: i32 },

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Max restarts exceeded ({0} crashes in 60 s)")]
    MaxRestarts(u32),

    #[error("Sidecar not spawned")]
    NotSpawned,

    #[error("IO error: {0}")]
    Io(String),
}

// ---------------------------------------------------------------------------
// Spawn helper
// ---------------------------------------------------------------------------

/// Spawn the Python sidecar and return (child, rx).
///
/// `rx` yields `CommandEvent` items (stdout lines, stderr lines, termination).
///
/// Resolution order for the Python interpreter:
///   1. `<script-dir>/.venv/bin/python3` — project-local venv
///   2. `python3` — system PATH
pub fn spawn_sidecar(
    app: &AppHandle,
    config_path: Option<&Path>,
) -> Result<
    (
        CommandChild,
        tokio::sync::mpsc::Receiver<tauri_plugin_shell::process::CommandEvent>,
    ),
    SttError,
> {
    let script_path = resolve_script_path(app);

    // Prefer a project-local venv over system python3
    let python_bin = std::path::PathBuf::from(&script_path)
        .parent()
        .map(|dir| dir.join(".venv").join("bin").join("python3"))
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "python3".into());

    let shell = app.shell();
    let mut args = vec!["-u".to_string(), script_path];
    if let Some(cp) = config_path {
        args.push("--config".to_string());
        args.push(cp.to_string_lossy().to_string());
    }
    let command = shell.command(&python_bin).args(&args);

    let (rx, child) = command.spawn().map_err(|e| SttError::Io(e.to_string()))?;

    Ok((child, rx))
}

// ---------------------------------------------------------------------------
// Sidecar instance
// ---------------------------------------------------------------------------

/// A running sidecar instance.  Holds the child handle for stdin writes/kill.
pub struct SidecarInstance {
    child: Option<CommandChild>,
    pub state: SidecarState,
    pub shutdown_flag: Arc<AtomicBool>,
    crash_count: u32,
    last_crash: Option<Instant>,
    backoff_s: u32,
    line_no: usize,
}

impl SidecarInstance {
    pub fn new(child: CommandChild) -> Self {
        Self {
            child: Some(child),
            state: SidecarState::Loading,
            shutdown_flag: Arc::new(AtomicBool::new(false)),
            crash_count: 0,
            last_crash: None,
            backoff_s: 1,
            line_no: 0,
        }
    }

    pub fn state(&self) -> &SidecarState {
        &self.state
    }

    pub fn shutdown_requested(&self) -> bool {
        self.shutdown_flag.load(Ordering::Relaxed)
    }

    pub fn request_shutdown(&self) {
        self.shutdown_flag.store(true, Ordering::Relaxed);
    }

    /// Send an audio chunk to stdin.
    pub fn send_audio(&mut self, pcm_bytes: &[u8]) -> Result<(), SttError> {
        let child = self.child.as_mut().ok_or(SttError::NotSpawned)?;

        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, pcm_bytes);

        let line = serde_json::json!({
            "type": "audio",
            "data": b64,
            "sample_rate": 16000,
        });

        let payload =
            serde_json::to_string(&line).map_err(|e| SttError::Protocol(e.to_string()))? + "\n";

        child
            .write(payload.as_bytes())
            .map_err(|e| SttError::Io(e.to_string()))?;

        self.state = SidecarState::Processing;
        Ok(())
    }

    /// Send a shutdown signal via stdin, then kill.
    pub fn shutdown(&mut self) {
        self.shutdown_flag.store(true, Ordering::Relaxed);
        if let Some(ref mut child) = self.child {
            let _ = child.write(b"{\"type\":\"shutdown\"}\n");
        }
        // Drop child to close stdin and kill the process
        self.child = None;
    }
}

// ---------------------------------------------------------------------------
// Background poll task
// ---------------------------------------------------------------------------

/// Run the sidecar stdout poll loop.
///
/// Spawn this as a `tokio::spawn` task after starting the sidecar.
/// On crash (exit ≠ 0), signals the orchestrator to re-spawn.
pub async fn run_poll_loop(
    app: AppHandle,
    mut rx: tokio::sync::mpsc::Receiver<CommandEvent>,
    sidecar: Arc<std::sync::Mutex<SidecarInstance>>,
    shutdown: Arc<AtomicBool>,
) {
    let mut line_no: usize = 0;

    while !shutdown.load(Ordering::Relaxed) {
        let event = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;

        match event {
            Ok(Some(CommandEvent::Stdout(bytes))) => {
                line_no += 1;
                let line = String::from_utf8_lossy(&bytes).trim().to_string();
                if line.is_empty() {
                    continue;
                }

                match serde_json::from_str::<SidecarMessage>(&line) {
                    Ok(SidecarMessage::Status { state, message }) => {
                        let mut inst = sidecar.lock().unwrap();
                        match state.as_str() {
                            "ready" => inst.state = SidecarState::Ready,
                            "loading" => inst.state = SidecarState::Loading,
                            "error" => inst.state = SidecarState::Fatal,
                            _ => {}
                        }
                        drop(inst);
                        events::emit_stt_status(&app, &state, message.as_deref());
                    }
                    Ok(SidecarMessage::Transcription {
                        text,
                        is_final,
                        timestamp,
                    }) => {
                        {
                            let mut inst = sidecar.lock().unwrap();
                            inst.state = SidecarState::Ready;
                        }
                        events::emit_subtitle_event(
                            &app,
                            events::SubtitleEventPayload {
                                id: timestamp as u32,
                                text,
                                is_final,
                                timestamp,
                            },
                        );
                    }
                    Err(e) => {
                        eprintln!("[stt] parse error line {line_no}: {e} — content: {line}");
                    }
                }
            }
            Ok(Some(CommandEvent::Stderr(bytes))) => {
                let line = String::from_utf8_lossy(&bytes);
                eprintln!("[stt:stderr] {}", line.trim());
            }
            Ok(Some(CommandEvent::Terminated(status))) => {
                let code = status.code.unwrap_or(-1);
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }
                eprintln!("[stt] sidecar exited with code {code}");
                match handle_sidecar_crash(&app, &sidecar, code).await {
                    Some(new_rx) => {
                        // Replace the old rx (now closed) with the new one from
                        // the re-spawned child.
                        rx = new_rx;
                    }
                    None => {
                        // Fatal — no more retries.  Exit the poll loop.
                        break;
                    }
                }
            }
            Ok(Some(CommandEvent::Error(err))) => {
                eprintln!("[stt] sidecar error: {err}");
            }
            Ok(Some(_)) => {
                // Unknown event variant (non-exhaustive enum) — ignore
            }
            Ok(None) => {
                // Channel closed
                break;
            }
            Err(_) => {
                // Timeout — loop back and check shutdown
                continue;
            }
        }
    }
}

/// Returns `Some(new_rx)` if the sidecar was re-spawned, or `None` if the
/// crash is fatal (model download failure, too many retries, or spawn error).
async fn handle_sidecar_crash(
    app: &AppHandle,
    sidecar: &Arc<std::sync::Mutex<SidecarInstance>>,
    exit_code: i32,
) -> Option<tokio::sync::mpsc::Receiver<CommandEvent>> {
    let now = Instant::now();

    // --- Phase 1: update state inside lock, then drop ---
    let exit_is_fatal = {
        let mut inst = sidecar.lock().unwrap();

        // Reset crash window if > 60 s since last crash
        if let Some(last) = inst.last_crash {
            if now.duration_since(last) > Duration::from_secs(60) {
                inst.crash_count = 0;
                inst.backoff_s = 1;
            }
        }

        inst.crash_count += 1;
        inst.last_crash = Some(now);

        if exit_code == 2 {
            inst.state = SidecarState::Fatal;
            true // fatal — no retry
        } else if inst.crash_count >= 5 {
            inst.state = SidecarState::Fatal;
            true // fatal — max retries
        } else {
            let wait_s = inst.backoff_s;
            inst.backoff_s = (inst.backoff_s * 2).min(30);
            inst.state = SidecarState::Crashed {
                attempt: inst.crash_count,
                backoff_s: wait_s,
            };
            false // will retry
        }
    }; // lock dropped here

    // --- Phase 2: emit event (no lock held) ---
    if exit_code == 2 {
        events::emit_stt_status(
            app,
            "error",
            Some("STT model unavailable — download failed. Restart the app to retry."),
        );
        return None;
    }

    if exit_is_fatal {
        events::emit_stt_status(
            app,
            "error",
            Some("STT crashed too many times — please restart"),
        );
        return None;
    }

    // Retry path: wait with backoff, then re-spawn
    let wait_s = {
        let inst = sidecar.lock().unwrap();
        let s = match &inst.state {
            SidecarState::Crashed { backoff_s, .. } => *backoff_s,
            _ => 1,
        };
        s
    };

    events::emit_stt_status(
        app,
        "error",
        Some(&format!(
            "Sidecar crashed (exit {exit_code}), restarting in {wait_s} s…"
        )),
    );

    tokio::time::sleep(Duration::from_secs(wait_s as u64)).await;

    // --- Phase 3: re-spawn ---
    match spawn_sidecar(app, None) {
        Ok((new_child, new_rx)) => {
            let mut inst = sidecar.lock().unwrap();
            inst.child = Some(new_child);
            inst.state = SidecarState::Loading;
            inst.line_no = 0;
            drop(inst);
            events::emit_stt_status(app, "loading", Some("Restarting sidecar…"));
            Some(new_rx)
        }
        Err(e) => {
            eprintln!("[stt] failed to re-spawn sidecar: {e}");
            let mut inst = sidecar.lock().unwrap();
            inst.state = SidecarState::Fatal;
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Script path resolution
// ---------------------------------------------------------------------------

#[allow(unused_variables)]
fn resolve_script_path(app: &AppHandle) -> String {
    #[cfg(debug_assertions)]
    {
        if let Ok(cwd) = std::env::current_dir() {
            let path = cwd
                .join("ai-pipeline")
                .join("stt")
                .join("whisper_stream.py");
            if path.exists() {
                return path.to_string_lossy().to_string();
            }
        }
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("ai-pipeline")
            .join("stt")
            .join("whisper_stream.py");
        path.to_string_lossy().to_string()
    }

    #[cfg(not(debug_assertions))]
    {
        app.path()
            .resource_dir()
            .ok()
            .map(|d| d.join("ai-pipeline").join("stt").join("whisper_stream.py"))
            .filter(|p| p.exists())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|exe| {
                        exe.parent()
                            .map(|p| p.join("ai-pipeline").join("stt").join("whisper_stream.py"))
                    })
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| "ai-pipeline/stt/whisper_stream.py".into())
            })
    }
}
