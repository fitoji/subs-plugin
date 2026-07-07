//! Tauri commands for Whisper settings management.
//!
//! Provides get/set/reload/reset commands consumed by both the native macOS
//! menu (via `on_menu_event`) and future frontend IPC.

use std::sync::{Arc, Mutex};

use tauri::Manager;
use tauri::State;

use crate::commands::AppState;
use crate::settings::{SettingsPath, WhisperSettings};
use crate::stt;

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Return a clone of the current Whisper settings.
#[tauri::command]
pub fn get_whisper_settings(
    settings: State<'_, Mutex<WhisperSettings>>,
) -> Result<WhisperSettings, String> {
    settings
        .lock()
        .map(|s| s.clone())
        .map_err(|e| e.to_string())
}

/// Update a single setting field by key+value, persist to TOML, rebuild menu.
#[tauri::command]
pub fn set_whisper_setting(
    app: tauri::AppHandle,
    settings: State<'_, Mutex<WhisperSettings>>,
    settings_path: State<'_, SettingsPath>,
    key: String,
    value: String,
) -> Result<(), String> {
    let mut s = settings.lock().map_err(|e| e.to_string())?;
    update_setting(&mut s, &key, &value)?;
    crate::settings::save(&settings_path.0, &s)?;
    drop(s);

    rebuild_whisper_menu(&app)?;
    Ok(())
}

/// Reload the STT sidecar: emit event, write temp config, kill + re-spawn.
#[tauri::command]
pub fn reload_whisper(
    app: tauri::AppHandle,
    settings: State<'_, Mutex<WhisperSettings>>,
    app_state: State<'_, AppState>,
) -> Result<(), String> {
    crate::events::emit_stt_status(&app, "reloading", Some("Reloading Whisper…"));

    let config_path = write_temp_config(&settings)?;

    // Shutdown old sidecar
    kill_sidecar(&app_state);

    // Brief wait for process to release resources
    std::thread::sleep(std::time::Duration::from_millis(300));

    spawn_new_sidecar(&app, &app_state, Some(&config_path))?;

    Ok(())
}

/// Reset settings to defaults, persist, rebuild menu, and reload sidecar.
#[tauri::command]
pub fn reset_whisper_settings(
    app: tauri::AppHandle,
    settings: State<'_, Mutex<WhisperSettings>>,
    settings_path: State<'_, SettingsPath>,
    app_state: State<'_, AppState>,
) -> Result<(), String> {
    // Reset to defaults and save
    {
        let mut s = settings.lock().map_err(|e| e.to_string())?;
        *s = WhisperSettings::default();
        crate::settings::save(&settings_path.0, &s)?;
    }

    rebuild_whisper_menu(&app)?;

    // Trigger reload
    crate::events::emit_stt_status(&app, "reloading", Some("Resetting Whisper…"));
    let config_path = write_temp_config(&settings)?;
    kill_sidecar(&app_state);
    std::thread::sleep(std::time::Duration::from_millis(300));
    spawn_new_sidecar(&app, &app_state, Some(&config_path))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Serialise current settings to a temporary JSON file for the sidecar.
fn write_temp_config(settings: &Mutex<WhisperSettings>) -> Result<std::path::PathBuf, String> {
    let s = settings.lock().map_err(|e| e.to_string())?;
    let temp_dir = std::env::temp_dir();
    let path = temp_dir.join(format!("whisper-config-{}.json", std::process::id()));
    let json =
        serde_json::to_string_pretty(&*s).map_err(|e| format!("Config serialization: {e}"))?;
    std::fs::write(&path, &json).map_err(|e| format!("Config write: {e}"))?;
    Ok(path)
}

/// Shutdown the old sidecar instance (if any).
fn kill_sidecar(app_state: &AppState) {
    let mut sc = app_state.sidecar.lock().unwrap();
    if let Some(sidecar_arc) = sc.take() {
        let mut inst = sidecar_arc.lock().unwrap();
        inst.shutdown();
    }
}

/// Spawn a new sidecar with an optional `--config` path and wire up the poll
/// loop.
fn spawn_new_sidecar(
    app: &tauri::AppHandle,
    app_state: &AppState,
    config_path: Option<&std::path::Path>,
) -> Result<(), String> {
    let (child, rx) = stt::spawn_sidecar(app, config_path)
        .map_err(|e| format!("Failed to spawn sidecar: {e}"))?;

    let instance = stt::SidecarInstance::new(child);
    let sidecar_arc = Arc::new(Mutex::new(instance));

    {
        let mut sc = app_state.sidecar.lock().unwrap();
        *sc = Some(sidecar_arc.clone());
    }

    let shutdown = app_state.sidecar_shutdown.clone();
    shutdown.store(false, std::sync::atomic::Ordering::Relaxed);

    let app_clone = app.clone();
    tokio::spawn(async move {
        stt::run_poll_loop(app_clone, rx, sidecar_arc, shutdown).await;
    });

    Ok(())
}

/// Update a single field on `WhisperSettings` from a string key+value.
///
/// The `key` matches the prefix used in menu-item IDs (e.g. `"temperature"`,
/// `"model"`, `"language"`, `"beam_size"`, `"no_speech_threshold"`, …).
pub fn update_setting(s: &mut WhisperSettings, key: &str, value: &str) -> Result<(), String> {
    match key {
        "temperature" => {
            let mid: f64 = value
                .parse()
                .map_err(|e| format!("Invalid temperature: {e}"))?;
            let spread = 0.2;
            s.temperature = ((mid - spread).max(0.0), mid, (mid + spread).min(1.0));
        }
        "beam_size" => {
            s.beam_size = value
                .parse()
                .map_err(|e| format!("Invalid beam_size: {e}"))?;
        }
        "model" => {
            s.model = match value {
                "tiny" => "mlx-community/whisper-tiny",
                "base" => "mlx-community/whisper-base",
                "small" => "mlx-community/whisper-small",
                "medium" => "mlx-community/whisper-medium",
                "large-v3-turbo" => "mlx-community/whisper-large-v3-turbo",
                _ => return Err(format!("Unknown model: {value}")),
            }
            .to_string();
        }
        "language" => {
            s.language = value.to_string();
        }
        "no_speech_threshold" => {
            s.no_speech_threshold = value
                .parse()
                .map_err(|e| format!("Invalid threshold: {e}"))?;
        }
        "compression_ratio_threshold" => {
            s.compression_ratio_threshold = value
                .parse()
                .map_err(|e| format!("Invalid compression_ratio: {e}"))?;
        }
        "logprob_threshold" => {
            s.logprob_threshold = value.parse().map_err(|e| format!("Invalid logprob: {e}"))?;
        }
        _ => return Err(format!("Unknown setting key: {key}")),
    }
    Ok(())
}

/// Map a short model name to its full HuggingFace repo identifier.
#[expect(dead_code)]
pub fn model_full_name(short: &str) -> &'static str {
    match short {
        "tiny" => "mlx-community/whisper-tiny",
        "base" => "mlx-community/whisper-base",
        "small" => "mlx-community/whisper-small",
        "medium" => "mlx-community/whisper-medium",
        "large-v3-turbo" => "mlx-community/whisper-large-v3-turbo",
        _ => "mlx-community/whisper-large-v3-turbo",
    }
}

/// Reverse: full model path → short name (for menu checkmark).
#[expect(dead_code)]
pub fn model_to_short(full: &str) -> &'static str {
    match full {
        "mlx-community/whisper-tiny" => "tiny",
        "mlx-community/whisper-base" => "base",
        "mlx-community/whisper-small" => "small",
        "mlx-community/whisper-medium" => "medium",
        "mlx-community/whisper-large-v3-turbo" => "large-v3-turbo",
        _ => "large-v3-turbo",
    }
}

/// Rebuild the entire Whisper menu to reflect the current settings state.
///
/// Called after any setting mutation so checkmarks stay in sync.
fn rebuild_whisper_menu(app: &tauri::AppHandle) -> Result<(), String> {
    let settings_state = app
        .try_state::<Mutex<WhisperSettings>>()
        .ok_or_else(|| "Settings not managed".to_string())?;
    let s = settings_state.lock().map_err(|e| e.to_string())?;
    let menu = crate::build_whisper_menu(app, &s).map_err(|e| e.to_string())?;
    app.set_menu(menu).map_err(|e| format!("Set menu: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Shared sidecar-reload for the menu event handler (which has no `State`).
// ---------------------------------------------------------------------------

/// Reload the sidecar using `try_state` (for non-command contexts such as
/// `on_menu_event` where `State<T>` is not available).
pub fn reload_sidecar_from_app(app: &tauri::AppHandle) -> Result<(), String> {
    crate::events::emit_stt_status(app, "reloading", Some("Reloading Whisper…"));

    let settings_state = app
        .try_state::<Mutex<WhisperSettings>>()
        .ok_or_else(|| "Settings not managed".to_string())?;
    let config_path = write_temp_config(&settings_state)?;

    let app_state = app
        .try_state::<AppState>()
        .ok_or_else(|| "AppState not managed".to_string())?;

    kill_sidecar(&app_state);
    std::thread::sleep(std::time::Duration::from_millis(300));
    spawn_new_sidecar(app, &app_state, Some(&config_path))?;

    Ok(())
}
