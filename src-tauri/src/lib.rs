mod window;

/// Run the Tauri application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // Apply window configuration on startup
            window::configure_window(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Subtitle Overlay");
}
