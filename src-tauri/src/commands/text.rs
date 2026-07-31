//! The text-processing stores: dictionary, snippets, per-app styles and
//! voice commands. Each is a load/save pair over a JSON file.
//!
//! All four go through `Store`, which holds the value and its path together, so
//! a save cannot be pointed at the wrong file and the write happens under the
//! same lock as the change.

use crate::{appdetect, dictionary, snippets, voicecommand, AppState};
use tauri::Manager;

#[tauri::command]
pub fn get_dictionary(state: tauri::State<AppState>) -> Vec<dictionary::DictEntry> {
    state.dict.with(|d| d.entries.clone())
}

#[tauri::command]
pub async fn set_dictionary(
    app: tauri::AppHandle,
    entries: Vec<dictionary::DictEntry>,
) -> Result<(), String> {
    let count = entries.len();
    let (hotwords, models_dir) = {
        let state = app.state::<AppState>();
        state
            .dict
            .update(|d| d.entries = entries, |d, p| d.save(p))?;
        let hw = state.dict.with(|d| d.hotwords());
        let dir = std::path::PathBuf::from(state.models_dir.lock().unwrap().clone());
        (hw, dir)
    };
    log::info!("Dictionary saved: {} entries", count);

    // Models that read bias phrases per utterance pick this up on the next
    // dictation for free. Models that read them at construction, currently
    // Qwen3, would go on using the phrases they were built with, so the file is
    // rewritten and the engine rebuilt. Skipping this would make the dictionary
    // save successfully and then do nothing until restart, which is worse than
    // it not saving at all.
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<AppState>();
        for spec in crate::models::MODELS.iter().filter(|m| m.hotwords_at_load()) {
            if let Err(e) = spec.write_hotwords(&models_dir, hotwords.as_deref()) {
                log::warn!("Could not write hotwords for {}: {}", spec.id, e);
            }
        }
        if let Err(e) = state.engine.reload_for_hotwords() {
            log::warn!("Engine rebuild after dictionary change failed: {}", e);
        }
    })
    .await
    .map_err(|e| format!("Dictionary applied but the engine rebuild failed: {}", e))?;

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

/// The mode pinned by a voice command, if any. Returned as the mode's name so
/// the UI can say which one without resolving ids itself.
#[tauri::command]
pub fn get_pinned_mode(state: tauri::State<AppState>) -> Option<String> {
    let id = state.pinned_mode.lock().unwrap().clone()?;
    state.modes.with(|s| s.modes.iter().find(|m| m.id == id).map(|m| m.name.clone()))
}

/// Pin a mode by id, or clear the pin with None. A pin set by voice must be
/// clearable by hand: state the user cannot see and cannot undo is worse than
/// no feature.
#[tauri::command]
pub fn set_pinned_mode(state: tauri::State<AppState>, mode_id: Option<String>) {
    *state.pinned_mode.lock().unwrap() = mode_id.clone();
    match mode_id {
        Some(id) => log::info!("Mode pinned: {}", id),
        None => log::info!("Mode pin cleared; app matching resumes"),
    }
}

/// Reveal the debug-audio folder in the system file manager.
///
/// Recorded takes are only useful if the user can reach them: the workflow is
/// record, then write a .txt of what was said beside each wav, then run the
/// comparison tool. A path printed in a log nobody reads is not a workflow.
#[tauri::command]
pub fn open_debug_audio_folder() -> Result<String, String> {
    let dir = std::path::PathBuf::from(std::env::var("HOME").map_err(|_| "No HOME".to_string())?)
        .join("Documents")
        .join("Inkwell Debug Audio");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {}", dir.display(), e))?;

    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(target_os = "windows")]
    let opener = "explorer";
    #[cfg(target_os = "linux")]
    let opener = "xdg-open";

    std::process::Command::new(opener)
        .arg(&dir)
        .spawn()
        .map_err(|e| format!("Could not open the folder: {}", e))?;
    Ok(dir.to_string_lossy().into_owned())
}
