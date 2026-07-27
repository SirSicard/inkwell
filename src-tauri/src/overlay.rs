use tauri::{AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};
use tauri::webview::Color;

const OVERLAY_LABEL: &str = "overlay";
const OVERLAY_WIDTH: f64 = 97.0;
const OVERLAY_HEIGHT: f64 = 97.0;
/// Gap between the ink blob and the bottom of the usable screen area.
const OVERLAY_BOTTOM_MARGIN: f64 = 80.0;

/// Bottom-center of the monitor the cursor is on, in physical desktop
/// coordinates. Uses the work area, so the Dock, the taskbar and the macOS
/// menu bar / notch are excluded. Physical coords because logical ones are
/// per-monitor and ambiguous across a mixed-DPI desktop.
fn overlay_position_setting(app: &AppHandle) -> String {
    app.try_state::<crate::AppState>()
        .and_then(|s| s.settings.lock().ok().map(|g| g.overlay_position.clone()))
        .unwrap_or_else(|| "bottom-center".to_string())
}

fn placement_position(app: &AppHandle) -> Option<PhysicalPosition<i32>> {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())?;

    let scale = monitor.scale_factor();
    let area = monitor.work_area();

    let w = (OVERLAY_WIDTH * scale).round() as i32;
    let h = (OVERLAY_HEIGHT * scale).round() as i32;
    let margin = (OVERLAY_BOTTOM_MARGIN * scale).round() as i32;

    // Corner placements keep the same margin off both edges so the blob sits at
    // a consistent distance whichever corner it is in.
    let placement = overlay_position_setting(app);
    let (vertical, horizontal) = placement.split_once('-').unwrap_or(("bottom", "center"));

    let x = match horizontal {
        "left" => area.position.x + margin,
        "right" => area.position.x + area.size.width as i32 - w - margin,
        _ => area.position.x + (area.size.width as i32 - w) / 2,
    };
    let y = match vertical {
        "top" => area.position.y + margin,
        _ => area.position.y + area.size.height as i32 - h - margin,
    };
    Some(PhysicalPosition::new(x, y))
}

/// Honour the `show_overlay` setting. Defaults to on when state isn't wired up
/// yet, so a startup-order change can never silently kill the overlay.
fn overlay_enabled(app: &AppHandle) -> bool {
    app.try_state::<crate::AppState>()
        .map(|s| s.settings.lock().map(|g| g.show_overlay).unwrap_or(true))
        .unwrap_or(true)
}

/// Show the floating overlay window. Always shows on top of everything.
/// Positioned bottom-center of the monitor under the cursor.
pub fn show(app: &AppHandle) {
    if !overlay_enabled(app) {
        return;
    }

    if let Some(overlay) = app.get_webview_window(OVERLAY_LABEL) {
        // Re-place it: the user may have moved to another display since.
        if let Some(pos) = placement_position(app) {
            let _ = overlay.set_position(tauri::Position::Physical(pos));
        }
        let _ = overlay.show();
        log::info!("Overlay shown (existing)");
        return;
    }

    // Build hidden and position before showing. Any first-frame position the
    // builder takes is logical and monitor-relative, which lands in the wrong
    // place on a second display; showing after the move avoids the flash.
    #[cfg_attr(target_os = "macos", allow(unused_mut))]
    let mut builder = WebviewWindowBuilder::new(
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
    .visible(false)
    .background_color(Color(0, 0, 0, 0));

    // .transparent(true) is needed on Windows/Linux but doesn't exist on macOS builder
    #[cfg(not(target_os = "macos"))]
    {
        builder = builder.transparent(true);
    }

    match builder.build() {
        Ok(w) => {
            let _ = w.set_ignore_cursor_events(true);
            // Remove window shadow
            let _ = w.set_shadow(false);

            // macOS: set NSWindow to fully transparent.
            // The `cocoa` crate is deprecated in favour of objc2-app-kit, but it
            // still works and the migration touches every unsafe AppKit call in
            // the app (appdetect.rs too), so it is deferred to its own pass.
            #[cfg(target_os = "macos")]
            #[allow(deprecated)]
            {
                use cocoa::appkit::{NSColor, NSWindow};
                use cocoa::base::{id, nil};
                if let Ok(ns_win) = w.ns_window() {
                    let ns_win = ns_win as id;
                    unsafe {
                        let clear = NSColor::clearColor(nil);
                        ns_win.setBackgroundColor_(clear);
                        ns_win.setOpaque_(cocoa::base::NO);
                    }
                    log::info!("Overlay: macOS NSWindow set to transparent");
                }
            }

            match placement_position(app) {
                Some(pos) => {
                    let _ = w.set_position(tauri::Position::Physical(pos));
                    log::info!("Overlay positioned: {} ({}, {})", overlay_position_setting(app), pos.x, pos.y);
                }
                None => log::warn!("No monitor found; overlay left at its default position"),
            }

            let _ = w.show();
            log::info!("Overlay window created");
        }
        Err(e) => log::error!("Failed to create overlay: {}", e),
    }
}

/// Hide the overlay window.
pub fn hide(app: &AppHandle) {
    if let Some(overlay) = app.get_webview_window(OVERLAY_LABEL) {
        let _ = overlay.hide();
    }
}
