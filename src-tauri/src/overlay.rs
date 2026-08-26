use tauri::{AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};
use tauri::webview::Color;

/// The overlay window's label. Public because `streaming.rs` emits partials
/// to this window by name: as a second string literal over there, renaming
/// the window would compile fine and silently stop the words appearing.
pub const OVERLAY_LABEL: &str = "overlay";
const OVERLAY_WIDTH: f64 = 97.0;
const OVERLAY_HEIGHT: f64 = 97.0;
/// Width when live partials are on. The blob keeps its 97px square and the
/// words get the rest, so the overlay stays exactly as it was for everyone who
/// has the feature off.
const OVERLAY_WIDTH_PARTIALS: f64 = 560.0;
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

/// How wide the overlay is, given whether it has words to show.
///
/// Pure, and separate from the reader below, for the same reason
/// `pipeline::decide_transition` is: the rest is an AppHandle and a settings
/// lock that no unit test can produce, and this one number decides whether the
/// blob sits alone or with a sentence beside it.
pub fn width_for(show_partials: bool) -> f64 {
    if show_partials { OVERLAY_WIDTH_PARTIALS } else { OVERLAY_WIDTH }
}

/// Read at every `show`, not once at build, so toggling the setting takes
/// effect on the next dictation instead of the next launch. Defaults to the
/// narrow overlay if state is not wired up yet: the wrong width on a window
/// that exists beats a startup-order change silently widening it for everyone.
fn overlay_width(app: &AppHandle) -> f64 {
    let on = app
        .try_state::<crate::AppState>()
        .map(|s| s.settings.lock().map(|g| g.show_partials).unwrap_or(false))
        .unwrap_or(false);
    width_for(on)
}

fn placement_position(app: &AppHandle) -> Option<PhysicalPosition<i32>> {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())?;

    let scale = monitor.scale_factor();
    let area = monitor.work_area();

    let w = (overlay_width(app) * scale).round() as i32;
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
        // Resize before placing: the partials setting may have been toggled
        // since this window was built, and the placement maths reads the width
        // the window is supposed to have.
        let width = overlay_width(app);
        let size = tauri::LogicalSize::new(width, OVERLAY_HEIGHT);
        let _ = overlay.set_min_size(Some(size));
        let _ = overlay.set_max_size(Some(size));
        let _ = overlay.set_size(tauri::Size::Logical(size));
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
    .inner_size(overlay_width(app), OVERLAY_HEIGHT)
    .min_inner_size(overlay_width(app), OVERLAY_HEIGHT)
    .max_inner_size(overlay_width(app), OVERLAY_HEIGHT)
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

#[cfg(test)]
mod width_tests {
    use super::*;

    /// The overlay was a 97px square for its whole life before live preview,
    /// and it must still be exactly that for everyone who leaves the feature
    /// off. This is the assertion that a default install is untouched.
    #[test]
    fn the_collapsed_overlay_is_the_square_it_always_was() {
        assert_eq!(width_for(false), 97.0);
    }

    /// 560px is not a taste question: below roughly this, 96 characters of
    /// partial text (streaming::MAX_PARTIAL_CHARS) stops fitting in the two
    /// lines the panel clamps to, and words start disappearing off the bottom.
    #[test]
    fn the_expanded_overlay_leaves_room_for_the_words() {
        let w = width_for(true);
        assert!(w >= 480.0, "too narrow for two lines of partial text: {w}");
        // The blob still needs its own square, whatever is beside it.
        assert!(w > width_for(false) + 97.0);
    }

    /// The height never changes, so a partials overlay is a wider strip and
    /// not a different shape. Placement maths in `placement_position` assumes
    /// this when it puts the window in a corner.
    #[test]
    fn only_the_width_moves() {
        assert_eq!(OVERLAY_HEIGHT, 97.0);
    }
}
