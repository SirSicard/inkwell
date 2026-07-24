use arboard::Clipboard;
use enigo::{Enigo, Key, Keyboard, Settings};
use std::thread;
use std::time::Duration;

/// Write text to clipboard and simulate Ctrl+V (Windows/Linux) or Cmd+V (macOS) to paste.
pub fn paste_text(text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }

    // 1. Write to clipboard
    let mut clipboard = Clipboard::new()
        .map_err(|e| format!("Failed to access clipboard: {}", e))?;
    clipboard.set_text(text)
        .map_err(|e| format!("Failed to write to clipboard: {}", e))?;

    log::info!("Clipboard set: \"{}\"", text);

    // 2. Small delay to ensure clipboard is ready
    thread::sleep(Duration::from_millis(50));

    // 3. Simulate paste keystroke
    let mut enigo = Enigo::new(&Settings::default())
        .map_err(|e| format!("Failed to create Enigo: {}", e))?;

    #[cfg(target_os = "macos")]
    {
        enigo.key(Key::Meta, enigo::Direction::Press)
            .map_err(|e| format!("Key press failed: {}", e))?;
        enigo.key(Key::Unicode('v'), enigo::Direction::Click)
            .map_err(|e| format!("Key click failed: {}", e))?;
        enigo.key(Key::Meta, enigo::Direction::Release)
            .map_err(|e| format!("Key release failed: {}", e))?;
    }

    #[cfg(not(target_os = "macos"))]
    {
        enigo.key(Key::Control, enigo::Direction::Press)
            .map_err(|e| format!("Key press failed: {}", e))?;
        enigo.key(Key::Unicode('v'), enigo::Direction::Click)
            .map_err(|e| format!("Key click failed: {}", e))?;
        enigo.key(Key::Control, enigo::Direction::Release)
            .map_err(|e| format!("Key release failed: {}", e))?;
    }

    log::info!("Paste simulated");
    Ok(())
}
