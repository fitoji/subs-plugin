mod audio;
mod commands;
mod events;
mod stt;
mod window;

use commands::AppState;

/// Run the Tauri application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState::new())
        .setup(|app| {
            // Apply window configuration on startup
            window::configure_window(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::start_capture,
            commands::stop_capture,
            commands::get_pipeline_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Subtitle Overlay");
}
