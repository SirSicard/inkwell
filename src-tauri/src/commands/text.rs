//! The text-processing stores: dictionary, snippets, per-app styles and
//! voice commands. Each is a load/save pair over a JSON file.
//!
//! All four go through `Store`, which holds the value and its path together, so
//! a save cannot be pointed at the wrong file and the write happens under the
//! same lock as the change.

use crate::{appdetect, dictionary, snippets, voicecommand, AppState};

#[tauri::command]
pub fn get_dictionary(state: tauri::State<AppState>) -> Vec<dictionary::DictEntry> {
    state.dict.with(|d| d.entries.clone())
}

#[tauri::command]
pub fn set_dictionary(
    state: tauri::State<AppState>,
    entries: Vec<dictionary::DictEntry>,
) -> Result<(), String> {
    let count = entries.len();
    state
        .dict
        .update(|d| d.entries = entries, |d, p| d.save(p))?;
    log::info!("Dictionary saved: {} entries", count);
    Ok(())
}

#[tauri::command]
pub fn get_snippets(state: tauri::State<AppState>) -> Vec<snippets::Snippet> {
    state.snippet_store.with(|s| s.snippets.clone())
}

#[tauri::command]
pub fn save_snippets(
    state: tauri::State<AppState>,
    items: Vec<snippets::Snippet>,
) -> Result<(), String> {
    let count = items.len();
    state
        .snippet_store
        .update(|s| s.snippets = items, |s, p| s.save(p))?;
    log::info!("Snippets saved: {} items", count);
    Ok(())
}

#[tauri::command]
pub fn test_snippet_expansion(state: tauri::State<AppState>, text: String) -> String {
    state.snippet_store.with(|s| s.expand(&text))
}

#[tauri::command]
pub fn get_app_styles(state: tauri::State<AppState>) -> appdetect::AppStyleRules {
    state.app_styles.get()
}

#[tauri::command]
pub fn save_app_styles(
    state: tauri::State<AppState>,
    rules: appdetect::AppStyleRules,
) -> Result<(), String> {
    state.app_styles.replace(rules, |r, p| r.save(p))?;
    log::info!("App style rules saved");
    Ok(())
}

#[tauri::command]
pub fn get_voice_commands(state: tauri::State<AppState>) -> voicecommand::VoiceCommandStore {
    state.voice_commands.get()
}

#[tauri::command]
pub fn save_voice_commands(
    state: tauri::State<AppState>,
    store: voicecommand::VoiceCommandStore,
) -> Result<(), String> {
    state.voice_commands.replace(store, |s, p| s.save(p))?;
    log::info!("Voice commands saved");
    Ok(())
}

#[tauri::command]
pub fn get_modes(state: tauri::State<AppState>) -> crate::modes::ModeStore {
    state.modes.get()
}

#[tauri::command]
pub fn save_modes(
    state: tauri::State<AppState>,
    store: crate::modes::ModeStore,
) -> Result<(), String> {
    // A store with no modes would make resolve() panic, and there is no sensible
    // UI for "no way to write text". Refuse rather than persist it.
    if store.modes.is_empty() {
        return Err("At least one mode is required".to_string());
    }
    let count = store.modes.len();
    state.modes.replace(store, |s, p| s.save(p))?;
    log::info!("Modes saved: {} mode(s)", count);
    Ok(())
}

/// The frontmost app's identity, so the UI can offer "add the app I was just in"
/// instead of asking the user to know their own bundle identifiers.
#[tauri::command]
pub fn get_foreground_app() -> Option<String> {
    crate::appdetect::foreground_app_id()
}
