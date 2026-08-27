use tauri::{AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};
use tauri::webview::Color;

/// The overlay window's label. Public because `streaming.rs` emits partials
/// to this window by name: as a second string literal over there, renaming
/// the window would compile fine and silently stop the words appearing.
pub const OVERLAY_LABEL: &str = "overlay";
const OVERLAY_WIDTH: f64 = 97.0;
const OVERLAY_HEIGHT: f64 = 97.0;
/// Window size when live partials are on: the words sit in a strip *above* the
/// blob, and the window grows up and outward around it.
///
/// Beside the blob was the obvious layout and it was wrong. A wider window
/// placed by its own left edge moves the blob by half the extra width: turning
/// the feature on slid it 231pt to the left, off the centre it had occupied for
/// every previous version. Growing upward and symmetrically instead leaves the
/// blob at exactly the coordinates it had before, which is the only acceptable
/// answer for a always-on-top window the user has learned the position of.
///
/// It also removes a problem the sideways layout had no good answer for: in a
/// right-hand corner placement the words would have run off the screen, so the
/// panel would have had to flip sides and tell the webview which side it was
/// on. Above is symmetric, so every one of the six placements behaves the same.
const OVERLAY_WIDTH_PARTIALS: f64 = 560.0;
/// Height the partial strip adds above the blob: two lines of 14px text at
/// 1.35, its padding and border, plus the gap. Mirrors `#partial` in
/// public/overlay.html.
const OVERLAY_PARTIALS_EXTRA_HEIGHT: f64 = 75.0;
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

/// The window's size, given whether it has words to show.
///
/// Pure, and separate from the reader below, for the same reason
/// `pipeline::decide_transition` is: the rest is an AppHandle and a settings
/// lock that no unit test can produce.
pub fn size_for(show_partials: bool) -> (f64, f64) {
    if show_partials {
        (OVERLAY_WIDTH_PARTIALS, OVERLAY_HEIGHT + OVERLAY_PARTIALS_EXTRA_HEIGHT)
    } else {
        (OVERLAY_WIDTH, OVERLAY_HEIGHT)
    }
}

/// Where the window's top-left goes, given where the *blob* should sit.
///
/// This is the whole fix. The blob's position is computed from the 97pt square
/// it has always been, and the window is then hung around it: half the extra
/// width to each side, all of the extra height above. Callers place the blob;
/// the window follows.
pub fn origin_for(blob_x: i32, blob_y: i32, show_partials: bool, scale: f64) -> (i32, i32) {
    if !show_partials {
        return (blob_x, blob_y);
    }
    let dx = ((OVERLAY_WIDTH_PARTIALS - OVERLAY_WIDTH) / 2.0 * scale).round() as i32;
    let dy = (OVERLAY_PARTIALS_EXTRA_HEIGHT * scale).round() as i32;
    (blob_x - dx, blob_y - dy)
}

/// Read at every `show`, not once at build, so toggling the setting takes
/// effect on the next dictation instead of the next launch. Defaults to the
/// narrow overlay if state is not wired up yet: the wrong width on a window
/// that exists beats a startup-order change silently widening it for everyone.
fn partials_on(app: &AppHandle) -> bool {
    app.try_state::<crate::AppState>()
        .map(|s| s.settings.lock().map(|g| g.show_partials).unwrap_or(false))
        .unwrap_or(false)
}

fn overlay_size(app: &AppHandle) -> (f64, f64) {
    size_for(partials_on(app))
}

fn placement_position(app: &AppHandle) -> Option<PhysicalPosition<i32>> {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())?;

    let scale = monitor.scale_factor();
    let area = monitor.work_area();

    // The BLOB is what gets placed, using the 97pt square it has always been,
    // so this maths is byte for byte what it was before live partials existed.
    // The window is hung around the result afterwards.
    let w = (OVERLAY_WIDTH * scale).round() as i32;
    let h = (OVERLAY_HEIGHT * scale).round() as i32;
    let margin = (OVERLAY_BOTTOM_MARGIN * scale).round() as i32;

    // Corner placements keep the same margin off both edges so the blob sits at
    // a consistent distance whichever corner it is in.
    let placement = overlay_position_setting(app);
    let (vertical, horizontal) = placement.split_once('-').unwrap_or(("bottom", "center"));

    let blob_x = match horizontal {
        "left" => area.position.x + margin,
        "right" => area.position.x + area.size.width as i32 - w - margin,
        _ => area.position.x + (area.size.width as i32 - w) / 2,
    };
    let blob_y = match vertical {
        "top" => area.position.y + margin,
        _ => area.position.y + area.size.height as i32 - h - margin,
    };

    let (x, y) = origin_for(blob_x, blob_y, partials_on(app), scale);
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
        let (w, h) = overlay_size(app);
        let size = tauri::LogicalSize::new(w, h);
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
    .inner_size(overlay_size(app).0, overlay_size(app).1)
    .min_inner_size(overlay_size(app).0, overlay_size(app).1)
    .max_inner_size(overlay_size(app).0, overlay_size(app).1)
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
mod geometry_tests {
    use super::*;

    /// The overlay was a 97pt square for its whole life before live preview,
    /// and it must still be exactly that for everyone who leaves it off.
    #[test]
    fn the_collapsed_overlay_is_the_square_it_always_was() {
        assert_eq!(size_for(false), (97.0, 97.0));
    }

    /// **The bug this file exists to prevent.** Turning live preview on used to
    /// move the blob 231pt to the left, because the wider window was placed by
    /// its own left edge. Measured in a real log: bottom-center went from
    /// x=1631 to x=1168 on the same display, 463 physical pixels at 2x.
    ///
    /// The blob's coordinates must not depend on the setting.
    #[test]
    fn turning_partials_on_does_not_move_the_blob() {
        let (bx, by) = (1631, 1714);
        for scale in [1.0, 2.0] {
            let (off_x, off_y) = origin_for(bx, by, false, scale);
            let (on_x, on_y) = origin_for(bx, by, true, scale);

            // Window origin shifts, because the window is bigger...
            assert!(on_x < off_x && on_y < off_y);

            // ...but the blob inside it does not. Recover the blob from the
            // window origin the way the layout does: centred horizontally,
            // flush to the bottom.
            let (w, h) = size_for(true);
            let blob_from_on = (
                on_x + (((w - OVERLAY_WIDTH) / 2.0 * scale).round() as i32),
                on_y + (((h - OVERLAY_HEIGHT) * scale).round() as i32),
            );
            assert_eq!(blob_from_on, (off_x, off_y), "blob moved at scale {scale}");
        }
    }

    /// The strip goes above the blob, never beside it. Beside meant flipping
    /// sides in a right-hand corner and telling the webview which side it was
    /// on; above is symmetric and every placement behaves identically.
    #[test]
    fn the_partial_strip_is_taller_and_wider_but_grows_upward() {
        let (w, h) = size_for(true);
        assert!(w > size_for(false).0, "no room for words");
        assert!(h > size_for(false).1, "the strip has no height");
        // Only upward: the y offset accounts for all the extra height.
        let (_, on_y) = origin_for(0, 1000, true, 1.0);
        assert_eq!(1000 - on_y, (h - OVERLAY_HEIGHT) as i32);
    }

    /// 560pt is not a taste question: below roughly this, 96 characters of
    /// partial text (streaming::MAX_PARTIAL_CHARS) stops fitting in the two
    /// lines the strip clamps to, and words fall off the bottom.
    #[test]
    fn the_strip_leaves_room_for_the_words() {
        assert!(size_for(true).0 >= 480.0);
    }

    /// It must still fit above the blob in a top-anchored placement, where
    /// there is only OVERLAY_BOTTOM_MARGIN of room before the screen edge.
    #[test]
    fn the_strip_fits_under_the_top_margin() {
        assert!(
            OVERLAY_PARTIALS_EXTRA_HEIGHT < OVERLAY_BOTTOM_MARGIN,
            "a top-placed overlay would push its words off screen"
        );
    }
}
