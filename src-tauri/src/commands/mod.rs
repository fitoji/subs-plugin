//! Tauri command handlers for the subtitle pipeline.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::State;
use tokio::sync::mpsc;

use crate::audio::{AudioCapture, AudioConfig, AudioStatus, create_audio_stream};
use crate::stt;
use screencapturekit::stream::sc_stream::SCStream;

// ---------------------------------------------------------------------------
// Managed state
// ---------------------------------------------------------------------------

/// Application-wide shared state, registered via `manage()` in `lib.rs`.
pub struct AppState {
    /// Shared audio capture processor (Arc so the processing task can hold a copy).
    pub audio_capture: Mutex<Option<Arc<Mutex<AudioCapture>>>>,
    /// Sidecar instance handle.
    pub sidecar: Mutex<Option<Arc<Mutex<stt::SidecarInstance>>>>,
    /// Whether the pipeline is active.
    pub capture_active: AtomicBool,
    /// Signal that the sidecar poll loop should exit.
    pub sidecar_shutdown: Arc<AtomicBool>,
    /// The live SCStream (kept alive while capturing).
    pub stream: Mutex<Option<SCStream>>,
    /// JoinHandle for the audio processing task.
    pub process_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            audio_capture: Mutex::new(None),
            sidecar: Mutex::new(None),
            capture_active: AtomicBool::new(false),
            sidecar_shutdown: Arc::new(AtomicBool::new(false)),
            stream: Mutex::new(None),
            process_handle: Mutex::new(None),
        }
    }
}

// ---------------------------------------------------------------------------
// Pipeline status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct PipelineStatus {
    pub audio: String,
    pub stt: String,
    pub capture_active: bool,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Start the audio capture + STT pipeline.
#[tauri::command]
pub async fn start_capture(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if state.capture_active.load(Ordering::Relaxed) {
        return Ok(());
    }

    // ---- 1. Spawn Python sidecar ----
    let (child, rx) =
        stt::spawn_sidecar(&app).map_err(|e| format!("Failed to spawn sidecar: {e}"))?;

    // ---- 2. Create shared sidecar instance ----
    let instance = stt::SidecarInstance::new(child);
    let sidecar_arc = Arc::new(Mutex::new(instance));
    let shutdown_flag = state.sidecar_shutdown.clone();
    shutdown_flag.store(false, Ordering::Relaxed);

    {
        let mut sc = state.sidecar.lock().unwrap();
        *sc = Some(sidecar_arc.clone());
    }

    // ---- 3. Create audio channel + SCStream ----
    // The audio callback runs on an SC dispatch queue; we use a tokio channel
    // to bridge into the async world.
    let (audio_tx, mut audio_rx) = mpsc::channel::<Vec<f32>>(64);

    // SCShareableContent::get() and SCStream::start_capture() are blocking —
    // run them in spawn_blocking so we don't tie up the async runtime.
    let stream = tokio::task::spawn_blocking(move || create_audio_stream(audio_tx))
        .await
        .map_err(|e| format!("Stream creation task panicked: {e}"))?
        .map_err(|e| format!("Failed to create audio stream: {e}"))?;

    // ---- 4. Create AudioCapture ----
    let audio_cfg = AudioConfig::default();
    let mut capture = AudioCapture::new(audio_cfg);
    capture.start().map_err(|e| format!("Failed to start audio capture: {e}"))?;

    let capture_arc = Arc::new(Mutex::new(capture));

    {
        let mut ac = state.audio_capture.lock().unwrap();
        *ac = Some(capture_arc.clone());
    }

    // ---- 5. Store SCStream ----
    {
        let mut s = state.stream.lock().unwrap();
        *s = Some(stream);
    }

    state.capture_active.store(true, Ordering::Relaxed);

    // ---- 6. Spawn audio processing task ----
    // Reads raw f32 frames from the channel, runs VAD + resample via
    // AudioCapture, and forwards PCM chunks to the sidecar.
    let proc_capture = capture_arc.clone();
    let proc_sidecar = sidecar_arc.clone();
    let proc_shutdown = shutdown_flag.clone();

    let handle = tokio::spawn(async move {
        while !proc_shutdown.load(Ordering::Relaxed) {
            match audio_rx.recv().await {
                Some(frames) => {
                    let pcm = {
                        let mut cap = proc_capture.lock().unwrap();
                        cap.handle_audio_buffer(&frames)
                    };

                    if let Some(pcm_bytes) = pcm {
                        let mut sc = proc_sidecar.lock().unwrap();
                        if let Err(e) = sc.send_audio(&pcm_bytes) {
                            eprintln!("[audio] send_audio to sidecar failed: {e}");
                            break;
                        }
                    }
                }
                None => {
                    // Channel closed — stream dropped, exit.
                    break;
                }
            }
        }
        eprintln!("[audio] processing task exited");
    });

    {
        let mut ph = state.process_handle.lock().unwrap();
        *ph = Some(handle);
    }

    // ---- 7. Spawn sidecar poll task ----
    let app_clone = app.clone();
    let shutdown_clone = shutdown_flag.clone();
    tokio::spawn(async move {
        stt::run_poll_loop(app_clone, rx, sidecar_arc, shutdown_clone).await;
    });

    crate::events::emit_stt_status(&app, "loading", Some("Starting capture…"));
    Ok(())
}

/// Stop the audio capture + STT pipeline and return to demo mode.
#[tauri::command]
pub async fn stop_capture(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if !state.capture_active.load(Ordering::Relaxed) {
        return Ok(());
    }

    // ---- 1. Signal shutdown ----
    state.sidecar_shutdown.store(true, Ordering::Relaxed);

    // ---- 2. Abort audio processing task ----
    {
        let mut ph = state.process_handle.lock().unwrap();
        if let Some(h) = ph.take() {
            h.abort();
        }
    }

    // ---- 3. Drop SCStream (stops capture) ----
    {
        let mut s = state.stream.lock().unwrap();
        *s = None; // Drop the SCStream, which releases native resources.
    }

    // ---- 4. Stop and remove AudioCapture ----
    {
        let mut ac = state.audio_capture.lock().unwrap();
        if let Some(capture_arc) = ac.take() {
            if let Ok(mut capture) = capture_arc.lock() {
                let _ = capture.stop();
            }
        }
    }

    // ---- 5. Stop sidecar ----
    {
        let mut sc = state.sidecar.lock().unwrap();
        if let Some(sidecar_arc) = sc.take() {
            if let Ok(mut inst) = sidecar_arc.lock() {
                inst.shutdown();
            }
        }
    }

    state.capture_active.store(false, Ordering::Relaxed);
    crate::events::emit_stt_status(&app, "ready", Some("Capture stopped, demo mode active"));
    Ok(())
}

/// Get the current pipeline status.
#[tauri::command]
pub async fn get_pipeline_status(state: State<'_, AppState>) -> Result<PipelineStatus, String> {
    let audio = state
        .audio_capture
        .lock()
        .map_err(|e| e.to_string())?
        .as_ref()
        .map(|arc_capture| {
            let capture = arc_capture.lock().unwrap();
            match capture.audio_status {
                AudioStatus::Idle => "idle".into(),
                AudioStatus::Active => "active".into(),
                AudioStatus::Silence => "silence".into(),
                AudioStatus::Error(ref msg) => format!("error: {msg}"),
            }
        })
        .unwrap_or_else(|| "disabled".into());

    Ok(PipelineStatus {
        audio,
        stt: "disabled".into(),
        capture_active: state.capture_active.load(Ordering::Relaxed),
    })
}
