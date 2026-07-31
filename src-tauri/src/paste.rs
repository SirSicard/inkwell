use arboard::Clipboard;
use enigo::{Enigo, Key, Keyboard, Settings};
use std::thread;
use std::time::Duration;


/// Input settings that never raise a system dialog.
///
/// enigo's defaults set `open_prompt_to_get_permissions: true`, so constructing
/// it calls AXIsProcessTrustedWithOptions with the prompt flag, and macOS shows
/// "Inkwell would like to control this computer" whenever the process is not
/// trusted. That is once per paste, which means once per dictation, forever.
///
/// It also fires when Accessibility *looks* granted: the toggle stays on in
/// System Settings, but TCC keys the grant to the app's code identity, and an
/// ad-hoc signed build gets a new identity every time it is rebuilt or
/// replaced. Updating the app therefore invalidates the grant while leaving the
/// switch on, which is unfalsifiable from the user's side.
///
/// So: never prompt from the paste path. Construction fails cleanly instead,
/// the caller falls back to leaving the text on the clipboard, and the app asks
/// for the permission from onboarding and settings, where there is screen to
/// explain why. Developer ID signing is what makes the grant survive updates.
fn input_settings() -> Settings {
    Settings {
        open_prompt_to_get_permissions: false,
        ..Default::default()
    }
}

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
    let mut enigo = Enigo::new(&input_settings()).map_err(|e| match e {
        // Name the actual remedy. "Failed to create Enigo: NoPermission" tells
        // the user nothing they can act on, and this is the error they are most
        // likely to see, since it fires whenever the Accessibility grant does
        // not match the running binary.
        enigo::NewConError::NoPermission => "Accessibility permission is missing, \
             so Inkwell cannot paste. Grant it in System Settings, Privacy and \
             Security, Accessibility. If Inkwell is already listed there, remove \
             it with the minus button and add it again: the grant is tied to the \
             exact app it was given to, and updating the app replaces that."
            .to_string(),
        other => format!("Could not simulate input: {}", other),
    })?;

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
    // and kills the process on every single successful dictation, right after
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
/// paste fails with no error. Non-prompting by design, so call it from
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

/// Gap between sending Cmd+C and reading the clipboard. The copy travels
/// through the target app's run loop, so reading immediately returns whatever
/// was on the clipboard before.
const COPY_SETTLE_MS: u64 = 120;

/// Copy the frontmost app's current selection and return it.
///
/// There is no cross-platform API for "what is selected in another
/// application", so this does what a person would: press copy, then look at the
/// clipboard. Returns None when the clipboard did not change, which is the only
/// available signal that nothing was selected.
///
/// The caller gets the user's previous clipboard back in `restore`, and must
/// use it: leaving a stolen selection on the clipboard is a side effect nobody
/// asked for.
pub fn copy_selection() -> Result<(Option<String>, Option<String>), String> {
    let mut clipboard = Clipboard::new()
        .map_err(|e| format!("Failed to access clipboard: {}", e))?;
    let previous = clipboard.get_text().ok();

    // A sentinel makes "nothing was selected" distinguishable from "the
    // selection happens to equal the clipboard". Without it, editing the same
    // text twice in a row would look like an empty selection the second time.
    let sentinel = "\u{0}inkwell-no-selection\u{0}";
    let _ = clipboard.set_text(sentinel);
    thread::sleep(Duration::from_millis(CLIPBOARD_SETTLE_MS));

    send_copy_keystroke()?;
    thread::sleep(Duration::from_millis(COPY_SETTLE_MS));

    let copied = clipboard.get_text().ok();
    let selection = match copied {
        Some(t) if t != sentinel && !t.trim().is_empty() => Some(t),
        _ => None,
    };
    Ok((selection, previous))
}

/// Put back what the user had on the clipboard before `copy_selection`.
///
/// `None` means there was no text to restore, which happens when the clipboard
/// held an image or was empty. That case still needs clearing rather than
/// skipping: `copy_selection` wrote a sentinel, and if nothing was selected the
/// sentinel is still there, so returning early would leave the user's clipboard
/// holding an internal marker string.
///
/// An image on the clipboard cannot survive this technique at all, since
/// reading a selection means writing to the clipboard first. Clearing is the
/// honest end state rather than pretending otherwise.
pub fn restore_clipboard(previous: Option<String>) {
    let result = match previous {
        Some(prev) => Clipboard::new().and_then(|mut c| c.set_text(prev)),
        None => Clipboard::new().and_then(|mut c| c.clear()),
    };
    if let Err(e) = result {
        log::warn!("Failed to restore the clipboard: {}", e);
    }
}

fn send_copy_keystroke() -> Result<(), String> {
    let mut enigo = Enigo::new(&input_settings()).map_err(|e| match e {
        // Name the actual remedy. "Failed to create Enigo: NoPermission" tells
        // the user nothing they can act on, and this is the error they are most
        // likely to see, since it fires whenever the Accessibility grant does
        // not match the running binary.
        enigo::NewConError::NoPermission => "Accessibility permission is missing, \
             so Inkwell cannot paste. Grant it in System Settings, Privacy and \
             Security, Accessibility. If Inkwell is already listed there, remove \
             it with the minus button and add it again: the grant is tied to the \
             exact app it was given to, and updating the app replaces that."
            .to_string(),
        other => format!("Could not simulate input: {}", other),
    })?;

    #[cfg(target_os = "macos")]
    let modifier = Key::Meta;
    #[cfg(not(target_os = "macos"))]
    let modifier = Key::Control;

    // Raw keycode for the same reason V is: Key::Unicode('c') goes through
    // macOS Text Services, which asserts it is on the main thread and kills the
    // process from a worker. See send_paste_keystroke.
    #[cfg(target_os = "macos")]
    const C_KEY: Key = Key::Other(0x08); // kVK_ANSI_C
    #[cfg(not(target_os = "macos"))]
    const C_KEY: Key = Key::Unicode('c');

    enigo.key(modifier, enigo::Direction::Press)
        .map_err(|e| format!("Key press failed: {}", e))?;
    let click = enigo.key(C_KEY, enigo::Direction::Click);
    let release = enigo.key(modifier, enigo::Direction::Release);

    click.map_err(|e| format!("Key click failed: {}", e))?;
    release.map_err(|e| format!("Key release failed: {}", e))?;
    Ok(())
}
