mod audio;
mod commands;
mod events;
mod settings;
mod stt;
mod window;

use std::sync::Mutex;

use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{Manager, Wry};

use commands::AppState;
use settings::{SettingsPath, WhisperSettings};

// ---------------------------------------------------------------------------
// Menu building
// ---------------------------------------------------------------------------

/// Build the complete Whisper settings menu tree.
///
/// Every parameter item is built with an ID that encodes
/// `{setting_key}_{value}` so the event handler can parse it back.
/// Checkmarks reflect the current `settings`.
pub fn build_whisper_menu(
    app: &impl Manager<Wry>,
    settings: &WhisperSettings,
) -> Result<Menu<Wry>, tauri::Error> {
    // --- Top-level actions ---
    let reload = MenuItem::with_id(app, "reload", "Reload Whisper", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let sep3 = PredefinedMenuItem::separator(app)?;
    let reset = MenuItem::with_id(app, "reset", "Reset to Defaults", true, None::<&str>)?;

    // ===================== Model =====================
    let model_submenu = {
        let tiny = mk_check(
            app,
            "model_tiny",
            "tiny",
            settings.model == "mlx-community/whisper-tiny",
        )?;
        let base = mk_check(
            app,
            "model_base",
            "base",
            settings.model == "mlx-community/whisper-base",
        )?;
        let small = mk_check(
            app,
            "model_small",
            "small",
            settings.model == "mlx-community/whisper-small",
        )?;
        let medium = mk_check(
            app,
            "model_medium",
            "medium",
            settings.model == "mlx-community/whisper-medium",
        )?;
        let large = mk_check(
            app,
            "model_large-v3-turbo",
            "large-v3-turbo",
            settings.model == "mlx-community/whisper-large-v3-turbo",
        )?;
        Submenu::with_items(app, "Model", true, &[&tiny, &base, &small, &medium, &large])?
    };

    // ===================== Language =====================
    let lang_submenu = {
        let auto = mk_check(app, "lang_auto", "auto", settings.language == "auto")?;
        let en = mk_check(app, "lang_en", "en", settings.language == "en")?;
        let de = mk_check(app, "lang_de", "de", settings.language == "de")?;
        let es = mk_check(app, "lang_es", "es", settings.language == "es")?;
        let fr = mk_check(app, "lang_fr", "fr", settings.language == "fr")?;
        let it = mk_check(app, "lang_it", "it", settings.language == "it")?;
        let pt = mk_check(app, "lang_pt", "pt", settings.language == "pt")?;
        Submenu::with_items(
            app,
            "Language",
            true,
            &[&auto, &en, &de, &es, &fr, &it, &pt],
        )?
    };

    // ===================== Basic =====================
    let basic_submenu = {
        let temperature = build_temperature_submenu(app, settings)?;
        let beam_size = build_beam_size_submenu(app, settings)?;
        Submenu::with_items(app, "Basic", true, &[&temperature, &beam_size])?
    };

    // ===================== Advanced =====================
    let advanced_submenu = {
        let ns = build_threshold_submenu(
            app,
            "no_speech_threshold",
            "No-Speech Threshold",
            settings.no_speech_threshold,
            &[0.2, 0.35, 0.5, 0.75],
        )?;
        let cr = build_threshold_submenu(
            app,
            "compression_ratio_threshold",
            "Compression Ratio Threshold",
            settings.compression_ratio_threshold,
            &[2.0, 2.4, 3.0],
        )?;
        let lp = build_threshold_submenu(
            app,
            "logprob_threshold",
            "Logprob Threshold",
            settings.logprob_threshold,
            &[-1.0, -0.5, 0.0],
        )?;
        Submenu::with_items(app, "Advanced", true, &[&ns, &cr, &lp])?
    };

    // ===================== Whisper menu =====================
    let whisper_submenu = Submenu::with_items(
        app,
        "Whisper",
        true,
        &[
            &reload,
            &sep1,
            &model_submenu,
            &lang_submenu,
            &sep2,
            &basic_submenu,
            &advanced_submenu,
            &sep3,
            &reset,
        ],
    )?;

    Menu::with_items(app, &[&whisper_submenu])
}

// ---------------------------------------------------------------------------
// Submenu builders
// ---------------------------------------------------------------------------

fn build_temperature_submenu(
    app: &impl Manager<Wry>,
    settings: &WhisperSettings,
) -> Result<Submenu<Wry>, tauri::Error> {
    let midpoint = settings.temperature.1;
    let presets = [0.0, 0.2, 0.4, 0.6, 0.8];
    let mut items: Vec<CheckMenuItem<Wry>> = Vec::with_capacity(presets.len());
    for &val in &presets {
        let id = format!("temperature_{val}");
        let text = format!("{val:.1}");
        let checked = (val - midpoint).abs() < f64::EPSILON;
        items.push(mk_check(app, &id, &text, checked)?);
    }
    let refs: Vec<&dyn IsMenuItem<Wry>> = items.iter().map(|i| i as &dyn IsMenuItem<Wry>).collect();
    Submenu::with_items(app, "Temperature", true, &refs)
}

fn build_beam_size_submenu(
    app: &impl Manager<Wry>,
    settings: &WhisperSettings,
) -> Result<Submenu<Wry>, tauri::Error> {
    let presets = [1u32, 3, 5, 7];
    let mut items: Vec<CheckMenuItem<Wry>> = Vec::with_capacity(presets.len());
    for &val in &presets {
        let id = format!("beam_size_{val}");
        let text = val.to_string();
        let checked = val == settings.beam_size;
        items.push(mk_check(app, &id, &text, checked)?);
    }
    let refs: Vec<&dyn IsMenuItem<Wry>> = items.iter().map(|i| i as &dyn IsMenuItem<Wry>).collect();
    Submenu::with_items(app, "Beam Size", true, &refs)
}

/// Build a threshold submenu (No-Speech, Compression Ratio, Logprob).
fn build_threshold_submenu(
    app: &impl Manager<Wry>,
    key: &str,
    label: &str,
    current: f64,
    presets: &[f64],
) -> Result<Submenu<Wry>, tauri::Error> {
    let mut items: Vec<CheckMenuItem<Wry>> = Vec::with_capacity(presets.len());
    for &val in presets {
        let id = format!("{key}_{val}");
        let text = format!("{val}");
        let checked = (val - current).abs() < f64::EPSILON;
        items.push(mk_check(app, &id, &text, checked)?);
    }
    let refs: Vec<&dyn IsMenuItem<Wry>> = items.iter().map(|i| i as &dyn IsMenuItem<Wry>).collect();
    Submenu::with_items(app, label, true, &refs)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a checkable menu item with the given checkmark state.
fn mk_check(
    app: &impl Manager<Wry>,
    id: &str,
    text: &str,
    checked: bool,
) -> Result<CheckMenuItem<Wry>, tauri::Error> {
    CheckMenuItem::with_id(app, id, text, true, checked, None::<&str>)
}

/// Rebuild the menu on the app handle to reflect current settings.
fn rebuild_menu(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<Mutex<WhisperSettings>>() {
        if let Ok(s) = state.lock() {
            if let Ok(menu) = build_whisper_menu(app, &s) {
                let _ = app.set_menu(menu);
            }
        }
    }
}

/// Parse a menu-item ID into a `(setting_key, setting_value)` pair.
fn parse_menu_item_id(id: &str) -> Option<(&str, &str)> {
    if let Some(rest) = id.strip_prefix("model_") {
        Some(("model", rest))
    } else if let Some(rest) = id.strip_prefix("lang_") {
        Some(("language", rest))
    } else if let Some(rest) = id.strip_prefix("temperature_") {
        Some(("temperature", rest))
    } else if let Some(rest) = id.strip_prefix("beam_size_") {
        Some(("beam_size", rest))
    } else if let Some(rest) = id.strip_prefix("no_speech_threshold_") {
        Some(("no_speech_threshold", rest))
    } else if let Some(rest) = id.strip_prefix("compression_ratio_threshold_") {
        Some(("compression_ratio_threshold", rest))
    } else if let Some(rest) = id.strip_prefix("logprob_threshold_") {
        Some(("logprob_threshold", rest))
    } else {
        None
    }
}

/// Handle a "set_*" menu item click — parse the ID, update settings, save.
fn handle_setting_item(app: &tauri::AppHandle, id: &str) {
    let Some((key, value)) = parse_menu_item_id(id) else {
        return;
    };

    if let Some(state) = app.try_state::<Mutex<WhisperSettings>>() {
        if let Ok(mut s) = state.lock() {
            if commands::settings_commands::update_setting(&mut s, key, value).is_ok() {
                if let Some(path_state) = app.try_state::<SettingsPath>() {
                    let _ = settings::save(&path_state.0, &s);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Run
// ---------------------------------------------------------------------------

/// Run the Tauri application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState::new())
        .setup(|app| {
            // ---- Settings state ----
            let settings_path = dirs::data_dir()
                .map(|d| d.join("com.subtitle-overlay.app").join("settings.user"))
                .unwrap_or_else(|| {
                    eprintln!("[settings] no data dir, using temp dir fallback");
                    std::env::temp_dir()
                        .join("subtitle-overlay")
                        .join("settings.user")
                });

            // Ensure parent directory exists
            if let Some(parent) = settings_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            let s = settings::load_or_default(&settings_path);
            let settings = Mutex::new(s);
            app.manage(settings);
            app.manage(SettingsPath(settings_path));

            // ---- Build native menu ----
            let settings_state = app.state::<Mutex<WhisperSettings>>();
            let current = settings_state.lock().unwrap();
            let menu = build_whisper_menu(app.handle(), &current)?;
            app.set_menu(menu)?;
            drop(current);

            // ---- Window config ----
            window::configure_window(app)?;

            Ok(())
        })
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            match id {
                "reload" => {
                    let _ = commands::settings_commands::reload_sidecar_from_app(app);
                }
                "reset" => {
                    // Restore defaults
                    if let Some(state) = app.try_state::<Mutex<WhisperSettings>>() {
                        if let Ok(mut s) = state.lock() {
                            *s = WhisperSettings::default();
                            if let Some(path_state) = app.try_state::<SettingsPath>() {
                                let _ = settings::save(&path_state.0, &s);
                            }
                        }
                    }
                    rebuild_menu(app);
                    let _ = commands::settings_commands::reload_sidecar_from_app(app);
                }
                _ => {
                    handle_setting_item(app, id);
                    rebuild_menu(app);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::start_capture,
            commands::stop_capture,
            commands::get_pipeline_status,
            commands::settings_commands::get_whisper_settings,
            commands::settings_commands::set_whisper_setting,
            commands::settings_commands::reload_whisper,
            commands::settings_commands::reset_whisper_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Subtitle Overlay");
}
