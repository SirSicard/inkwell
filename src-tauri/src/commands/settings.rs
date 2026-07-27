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
