use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri::webview::Color;

const OVERLAY_LABEL: &str = "overlay";

/// Get bottom-center screen position for the overlay.
fn get_bottom_center_position() -> (f64, f64) {
    // Try to get primary monitor size; fall back to common resolution
    let (screen_w, screen_h) = (1920.0, 1080.0); // safe default
    let x = (screen_w - OVERLAY_WIDTH) / 2.0;
    let y = screen_h - OVERLAY_HEIGHT - 80.0; // 80px above taskbar
    (x, y)
}
const OVERLAY_WIDTH: f64 = 97.0;
const OVERLAY_HEIGHT: f64 = 97.0;

/// Show the floating overlay window. Always shows on top of everything.
/// Positioned bottom-center of the primary monitor.
pub fn show(app: &AppHandle) {
    if let Some(overlay) = app.get_webview_window(OVERLAY_LABEL) {
        let _ = overlay.show();
        log::info!("Overlay shown (existing)");
        return;
    }
    {
        // Calculate bottom-center position
        let (x, y) = get_bottom_center_position();

        // Create overlay window - transparent via background_color alpha=0
        match WebviewWindowBuilder::new(
            app,
            OVERLAY_LABEL,
            WebviewUrl::App("overlay.html".into()),
        )
        .title("Inkwell Recording")
        .inner_size(OVERLAY_WIDTH, OVERLAY_HEIGHT)
        .min_inner_size(OVERLAY_WIDTH, OVERLAY_HEIGHT)
        .max_inner_size(OVERLAY_WIDTH, OVERLAY_HEIGHT)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .transparent(true)
        .background_color(Color(0, 0, 0, 0))
        .position(x, y)
        .build()
        {
            Ok(w) => {
                let _ = w.set_ignore_cursor_events(true);
                // Remove window shadow
                let _ = w.set_shadow(false);

                // Position bottom-center
                if let Ok(monitor) = w.primary_monitor() {
                    if let Some(m) = monitor {
                        let size = m.size();
                        let scale = m.scale_factor();
                        let sw = size.width as f64 / scale;
                        let sh = size.height as f64 / scale;
                        let cx = (sw - OVERLAY_WIDTH) / 2.0;
                        let cy = sh - OVERLAY_HEIGHT - 80.0;
                        let _ = w.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(cx, cy)));
                        log::info!("Overlay positioned: bottom-center ({:.0}, {:.0})", cx, cy);
                    }
                }

                log::info!("Overlay window created");
            }
            Err(e) => log::error!("Failed to create overlay: {}", e),
        }
    }
}

/// Hide the overlay window.
pub fn hide(app: &AppHandle) {
    if let Some(overlay) = app.get_webview_window(OVERLAY_LABEL) {
        let _ = overlay.hide();
    }
}