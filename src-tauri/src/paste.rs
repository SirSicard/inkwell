use arboard::Clipboard;
use enigo::{Enigo, Key, Keyboard, Settings};
use std::thread;
use std::time::Duration;

/// Gap between writing the clipboard and sending the paste keystroke.
/// Load-bearing: both the macOS pasteboard and the Windows OLE clipboard
/// publish asynchronously to the calling process, so a keystroke sent in the
/// same tick can paste the *previous* contents.
const CLIPBOARD_SETTLE_MS: u64 = 50;

/// Gap between the paste keystroke and putting the user's clipboard back.
/// Load-bearing: the target app reads the clipboard on its own run loop after
/// it handles the key event. Restoring sooner races that read and pastes the
/// old contents instead of the transcript.
const CLIPBOARD_RESTORE_MS: u64 = 300;

/// Write text to clipboard and simulate Ctrl+V (Windows/Linux) or Cmd+V (macOS)
/// to paste, then restore whatever the user had on the clipboard before.
pub fn paste_text(text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }

    let mut clipboard = Clipboard::new()
        .map_err(|e| format!("Failed to access clipboard: {}", e))?;

    // Text only: arboard cannot round-trip images or custom flavours, so if the
    // clipboard held one we leave it replaced rather than silently destroy it
    // by writing back an empty string.
    let previous = clipboard.get_text().ok();

    clipboard.set_text(text)
        .map_err(|e| format!("Failed to write to clipboard: {}", e))?;

    log::info!("Clipboard set ({} chars)", text.chars().count());

    thread::sleep(Duration::from_millis(CLIPBOARD_SETTLE_MS));

    let result = send_paste_keystroke();

    thread::sleep(Duration::from_millis(CLIPBOARD_RESTORE_MS));
    if let Some(prev) = previous {
        if let Err(e) = clipboard.set_text(prev) {
            log::warn!("Failed to restore previous clipboard: {}", e);
        }
    }

    result?;
    log::info!("Paste simulated");
    Ok(())
}

fn send_paste_keystroke() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default())
        .map_err(|e| format!("Failed to create Enigo: {}", e))?;

    #[cfg(target_os = "macos")]
    let modifier = Key::Meta;
    #[cfg(not(target_os = "macos"))]
    let modifier = Key::Control;

    // The V key, by raw virtual keycode rather than by character.
    //
    // Key::Unicode('v') looks tempting, but on macOS enigo resolves it through
    // get_layoutdependent_keycode, which calls the Text Services Manager
    // (TSMGetInputSourceProperty). TSM asserts it is on the main thread, and
    // this runs on the pipeline's worker thread, so libdispatch raises SIGTRAP
    // and kills the process — every single successful dictation, right after
    // the clipboard write. It is not a Rust panic, so catch_unwind never sees
    // it. The raw keycode skips that lookup entirely (and skips the 256 TSM
    // calls the layout scan costs per paste).
    #[cfg(target_os = "macos")]
    const V_KEY: Key = Key::Other(0x09); // kVK_ANSI_V
    #[cfg(not(target_os = "macos"))]
    const V_KEY: Key = Key::Unicode('v');

    enigo.key(modifier, enigo::Direction::Press)
        .map_err(|e| format!("Key press failed: {}", e))?;
    let click = enigo.key(V_KEY, enigo::Direction::Click);
    // Release the modifier even if the click failed, or the user is left with a
    // stuck Cmd/Ctrl.
    let release = enigo.key(modifier, enigo::Direction::Release);

    click.map_err(|e| format!("Key click failed: {}", e))?;
    release.map_err(|e| format!("Key release failed: {}", e))?;
    Ok(())
}

/// macOS: does this process hold Accessibility permission?
///
/// Without it the synthetic Cmd+V is swallowed by the window server and the
/// paste fails with no error. Non-prompting by design — call it from
/// onboarding/settings so the user can be guided, never on the paste path.
#[cfg(target_os = "macos")]
#[tauri::command]
pub fn check_accessibility_permission() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        // Returns a Boolean (unsigned char), not a Rust bool.
        fn AXIsProcessTrusted() -> u8;
    }
    unsafe { AXIsProcessTrusted() != 0 }
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn check_accessibility_permission() -> bool {
    true
}

/// macOS: open System Settings on the Accessibility privacy pane so the user
/// can grant the permission `check_accessibility_permission` reports missing.
#[cfg(target_os = "macos")]
#[tauri::command]
pub fn open_accessibility_settings() -> Result<(), String> {
    std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .spawn()
        .map_err(|e| format!("Failed to open System Settings: {}", e))?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn open_accessibility_settings() -> Result<(), String> {
    Ok(())
}
