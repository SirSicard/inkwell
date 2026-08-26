//! Settings, style, hotkey and first-run state.

use crate::{settings, style, AppState};

#[tauri::command]
pub fn set_style(state: tauri::State<AppState>, style_name: String) -> Result<(), String> {
    let s: style::Style = serde_json::from_str(&format!("\"{}\"", style_name))
        .map_err(|_| format!("Unknown style: {}", style_name))?;
    log::info!("Style set to: {:?}", s);
    *state.style.lock().unwrap() = s;

    let mut settings = state.settings.lock().unwrap();
    settings.style = style_name;
    let path = state.settings_path.lock().unwrap().clone();
    let _ = settings.save(std::path::Path::new(&path));
    Ok(())
}

#[tauri::command]
pub fn get_style(state: tauri::State<AppState>) -> String {
    let s = state.style.lock().unwrap();
    match *s {
        style::Style::Formal => "formal".to_string(),
        style::Style::Casual => "casual".to_string(),
        style::Style::Relaxed => "relaxed".to_string(),
    }
}

/// Pause or resume the recording in progress. Returns the new paused state.
///
/// Refuses when nothing is recording rather than silently setting a flag that
/// the next recording would inherit: a stale pause would make the following
/// dictation capture nothing at all, which is the worst failure this feature
/// could introduce.
#[tauri::command]
pub fn toggle_pause(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<bool, String> {
    use std::sync::atomic::Ordering;
    use tauri::Emitter;

    let guard = state.audio.lock().unwrap();
    let audio = guard.as_ref().ok_or("No audio device")?;

    if !audio.is_recording.load(Ordering::Relaxed) {
        return Err("Nothing is recording".to_string());
    }

    let paused = !audio.is_paused.load(Ordering::Relaxed);
    audio.is_paused.store(paused, Ordering::Relaxed);
    log::info!("Recording {}", if paused { "paused" } else { "resumed" });
    let _ = app.emit("recording-paused", paused);
    Ok(paused)
}

#[tauri::command]
pub fn get_settings(state: tauri::State<AppState>) -> settings::Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
pub fn update_settings(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    key: String,
    value: String,
) -> Result<(), String> {
    // The mic can only be swapped once the new choice is persisted and the
    // settings lock is released; the arm records it and the tail applies it.
    let mut new_mic: Option<String> = None;
    let mut settings = state.settings.lock().unwrap();
    match key.as_str() {
        "style" => settings.style = value,
        "model" => settings.model = value,
        "hotkey" => settings.hotkey = value,
        "recording_mode" => settings.recording_mode = value,
        "start_on_boot" => {
            settings.start_on_boot = value == "true";
            crate::setup::apply_start_on_boot(&app, settings.start_on_boot);
        }
        "show_overlay" => settings.show_overlay = value == "true",
        "overlay_position" => settings.overlay_position = value,
        "theme" => settings.theme = value,
        "advanced_mode" => settings.advanced_mode = value == "true",
        "mic_device" => {
            settings.mic_device = value.clone();
            new_mic = Some(value);
        }
        "vad_threshold" => settings.vad_threshold = value.parse().unwrap_or(0.5),
        "mic_idle_release_mins" => {
            settings.mic_idle_release_mins = value.parse().unwrap_or(3)
        }
        "append_space" => settings.append_space = value == "true",
        "sound_dictation" => {
            settings.sound_dictation = value == "true";
            crate::sounds::set_dictation_sounds(settings.sound_dictation);
        }
        "debug_save_audio" => settings.debug_save_audio = value == "true",
        "remove_fillers" => settings.remove_fillers = value == "true",
        _ => return Err(format!("Unknown setting: {}", key)),
    }
    let path = state.settings_path.lock().unwrap().clone();
    settings.save(std::path::Path::new(&path))?;
    drop(settings);

    // Save first, then reopen the capture stream: the setting is persisted even
    // when the live switch is refused ("Cannot switch microphone while
    // recording"), and the caller sees that refusal instead of a silent no-op.
    if let Some(device) = new_mic {
        crate::audio::restart_audio_capture(&app, &device)?;
    }
    Ok(())
}

#[tauri::command]
pub fn check_first_run(state: tauri::State<AppState>) -> bool {
    let mut first = state.is_first_run.lock().unwrap();
    let val = *first;
    *first = false;
    val
}

#[tauri::command]
pub fn set_hotkey(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    hotkey: String,
) -> Result<(), String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;

    let shortcut: tauri_plugin_global_shortcut::Shortcut = hotkey
        .parse()
        .map_err(|e| format!("Invalid hotkey '{}': {}", hotkey, e))?;

    let manager = app.global_shortcut();
    let _ = manager.unregister_all();

    manager
        .register(shortcut)
        .map_err(|e| format!("Failed to register '{}': {}", hotkey, e))?;

    // unregister_all took the edit hotkey with it. Without this, changing the
    // dictation hotkey silently turned voice editing off until the next
    // restart, which is the kind of failure nobody connects to what they did.
    let edit_hotkey = state.settings.lock().unwrap().edit_hotkey.clone();
    if !edit_hotkey.trim().is_empty() {
        if let Ok(sc) = edit_hotkey.parse::<tauri_plugin_global_shortcut::Shortcut>() {
            if sc != shortcut {
                if let Err(e) = manager.register(sc) {
                    log::warn!("Could not re-register the edit hotkey: {}", e);
                }
            }
        }
    }

    let mut settings = state.settings.lock().unwrap();
    settings.hotkey = hotkey.clone();
    let path = state.settings_path.lock().unwrap().clone();
    let _ = settings.save(std::path::Path::new(&path));

    log::info!("Hotkey changed to: {}", hotkey);
    Ok(())
}

#[tauri::command]
pub fn get_vad_threshold(state: tauri::State<AppState>) -> f32 {
    state.settings.lock().unwrap().vad_threshold
}

#[tauri::command]
pub fn set_vad_threshold(state: tauri::State<AppState>, threshold: f32) -> Result<(), String> {
    let mut settings = state.settings.lock().unwrap();
    settings.vad_threshold = threshold.clamp(0.1, 0.95);
    let path = state.settings_path.lock().unwrap().clone();
    settings.save(std::path::Path::new(&path))?;
    log::info!("VAD threshold set to: {:.2}", settings.vad_threshold);
    Ok(())
}

/// Change the voice-edit hotkey, or disable the feature with an empty string.
///
/// Registers before persisting, so a combination the OS refuses leaves the
/// working one in place instead of saving a setting that does nothing.
#[tauri::command]
pub fn set_edit_hotkey(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    hotkey: String,
) -> Result<(), String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;

    let manager = app.global_shortcut();
    let dictation = state.settings.lock().unwrap().hotkey.clone();
    let dictation_sc = dictation.parse::<tauri_plugin_global_shortcut::Shortcut>().ok();

    let old = state.settings.lock().unwrap().edit_hotkey.clone();
    if let Ok(sc) = old.parse::<tauri_plugin_global_shortcut::Shortcut>() {
        if Some(&sc) != dictation_sc.as_ref() {
            let _ = manager.unregister(sc);
        }
    }

    let trimmed = hotkey.trim().to_string();
    if !trimmed.is_empty() {
        let sc: tauri_plugin_global_shortcut::Shortcut = trimmed
            .parse()
            .map_err(|e| format!("Invalid hotkey '{}': {}", trimmed, e))?;
        if Some(&sc) == dictation_sc.as_ref() {
            return Err("That is already the dictation hotkey.".to_string());
        }
        manager
            .register(sc)
            .map_err(|e| format!("Failed to register '{}': {}", trimmed, e))?;
    }

    let mut settings = state.settings.lock().unwrap();
    settings.edit_hotkey = trimmed.clone();
    let path = state.settings_path.lock().unwrap().clone();
    let _ = settings.save(std::path::Path::new(&path));
    log::info!(
        "Edit hotkey set to {}",
        if trimmed.is_empty() { "(disabled)" } else { &trimmed }
    );
    Ok(())
}

#[cfg(test)]
mod hotkey_parse_tests {
    use tauri_plugin_global_shortcut::Shortcut;

    /// Single-key hotkeys are a frontend policy, but they only work if the
    /// plugin's parser accepts a bare key at all. Locked here so a plugin
    /// upgrade that changes parsing fails loudly instead of turning the
    /// feature into a save-time error.
    #[test]
    fn bare_function_keys_parse() {
        for k in ["f1", "f5", "f13", "f19", "f24"] {
            assert!(k.parse::<Shortcut>().is_ok(), "'{k}' failed to parse");
        }
    }

    #[test]
    fn bare_safe_extras_parse() {
        for k in ["insert", "pause", "scrolllock"] {
            assert!(k.parse::<Shortcut>().is_ok(), "'{k}' failed to parse");
        }
    }

    #[test]
    fn existing_combos_still_parse() {
        for k in ["shift+super+space", "ctrl+space", "super+shift+e"] {
            assert!(k.parse::<Shortcut>().is_ok(), "'{k}' failed to parse");
        }
    }
}
