use tauri::{App, Manager};

/// Configure the main window for the overlay.
///
/// Sets the window to always stay on top of other applications
/// and positions it at the bottom-center of the screen.
/// (transparent, decorations, etc. are configured in tauri.conf.json)
pub fn configure_window(app: &App) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(window) = app.get_webview_window("main") {
        // Get the primary monitor's available size
        if let Some(monitor) = window.current_monitor()? {
            let size = monitor.size();
            let window_size = window.outer_size()?;

            // Position at bottom-center
            let x = (size.width.saturating_sub(window_size.width)) / 2;
            let y = size.height.saturating_sub(window_size.height) - 80; // 80px from bottom

            window.set_position(tauri::PhysicalPosition::new(x, y))?;
        }
    }
    Ok(())
}
