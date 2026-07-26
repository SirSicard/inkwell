use crate::AppState;
use crate::{
    appdetect, dictionary, export, filetranscribe, history, models, settings, snippets,
    style, voicecommand,
};
use tauri::{Emitter, Manager};

#[tauri::command]
pub fn get_model_name(state: tauri::State<AppState>) -> String {
    state.model_name.lock().unwrap().clone()
}

/// The model catalogue with installed state, so the UI does not keep its own
/// copy of the list (it used to, and the two drifted).
#[tauri::command]
pub fn list_models(state: tauri::State<AppState>) -> Vec<models::ModelInfo> {
    let models_dir = state.models_dir.lock().unwrap().clone();
    models::catalog(std::path::Path::new(&models_dir))
}

#[tauri::command]
pub fn switch_model(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    model: String,
) -> Result<String, String> {
    let models_dir = state.models_dir.lock().unwrap().clone();
    let models_path = std::path::Path::new(&models_dir);

    let spec = models::find(&model).ok_or_else(|| format!("Unknown model: {}", model))?;
    let new_engine = spec.load(models_path)?;

    // Name comes from the spec, not from ModelType. Deriving it here as well
    // gave two sources for the same string, and they disagreed on casing
    // ("Whisper turbo" vs "Whisper Turbo"), which silently broke the
    // active-model check in remove_model.
    let name_str = spec.display.to_string();

    *state.model_name.lock().unwrap() = name_str.clone();
    *state.engine.lock().unwrap() = Some(new_engine);
    let _ = app.emit("model-loaded", &name_str);
    log::info!("Switched to model: {}", name_str);

    let mut settings = state.settings.lock().unwrap();
    settings.model = model;
    let path = state.settings_path.lock().unwrap().clone();
    let _ = settings.save(std::path::Path::new(&path));

    Ok(name_str)
}

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
pub fn get_transcripts(
    state: tauri::State<AppState>,
    limit: Option<usize>,
) -> Result<Vec<history::Transcript>, String> {
    let db_guard = state.db.lock().unwrap();
    match db_guard.as_ref() {
        Some(db) => db.recent(limit.unwrap_or(50)),
        None => Ok(vec![]),
    }
}

#[tauri::command]
pub fn search_transcripts(
    state: tauri::State<AppState>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<history::Transcript>, String> {
    let db_guard = state.db.lock().unwrap();
    match db_guard.as_ref() {
        Some(db) => db.search(&query, limit.unwrap_or(50)),
        None => Ok(vec![]),
    }
}

#[tauri::command]
pub fn delete_transcript(state: tauri::State<AppState>, id: i64) -> Result<(), String> {
    let db_guard = state.db.lock().unwrap();
    match db_guard.as_ref() {
        Some(db) => db.delete(id),
        None => Err("No database".to_string()),
    }
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

#[tauri::command]
pub fn get_dictionary(state: tauri::State<AppState>) -> Vec<dictionary::DictEntry> {
    state.dict.lock().unwrap().entries.clone()
}

#[tauri::command]
pub fn set_dictionary(
    state: tauri::State<AppState>,
    entries: Vec<dictionary::DictEntry>,
) -> Result<(), String> {
    let mut dict = state.dict.lock().unwrap();
    dict.entries = entries;
    let path = state.dict_path.lock().unwrap().clone();
    dict.save(std::path::Path::new(&path))?;
    log::info!("Dictionary saved: {} entries", dict.entries.len());
    Ok(())
}

#[tauri::command]
pub fn export_transcripts(
    state: tauri::State<AppState>,
    format: String,
    ids: Vec<i64>,
) -> Result<String, String> {
    let db_guard = state.db.lock().unwrap();
    let db = db_guard.as_ref().ok_or("Database not initialized")?;
    let transcripts = if ids.is_empty() {
        db.recent(10_000)?
    } else {
        let all = db.recent(10_000)?;
        all.into_iter().filter(|t| ids.contains(&t.id)).collect()
    };

    let content = match format.as_str() {
        "txt" => export::to_txt(&transcripts),
        "srt" => export::to_srt(&transcripts),
        "json" => export::to_json(&transcripts),
        "csv" => export::to_csv(&transcripts),
        other => return Err(format!("Unknown format: {}", other)),
    };

    log::info!(
        "Exported {} transcripts as {}",
        transcripts.len(),
        format
    );
    Ok(content)
}

/// Transcribe an audio/video file. Decodes, runs VAD, chunks, and transcribes.
/// Emits progress events: file-transcribe-progress { phase, percent, text? }
#[tauri::command]
pub async fn transcribe_file(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<serde_json::Value, String> {
    use serde_json::json;

    let file_path = std::path::Path::new(&path);
    if !file_path.exists() {
        return Err(format!("File not found: {}", path));
    }
    if !filetranscribe::is_supported(file_path) {
        return Err(format!(
            "Unsupported format. Supported: {}",
            filetranscribe::SUPPORTED_EXTENSIONS.join(", ")
        ));
    }

    let filename = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    // Phase 1: Decode
    let _ = app.emit(
        "file-transcribe-progress",
        json!({ "phase": "decoding", "percent": 5, "filename": &filename }),
    );
    let samples = filetranscribe::decode_to_pcm(file_path)?;
    let duration_s = samples.len() as f32 / 16000.0;

    let _ = app.emit(
        "file-transcribe-progress",
        json!({ "phase": "decoding", "percent": 15, "filename": &filename }),
    );

    // Phase 2: VAD chunking
    let _ = app.emit(
        "file-transcribe-progress",
        json!({ "phase": "analyzing", "percent": 18, "filename": &filename }),
    );
    let vad_path = state.vad_model_path.lock().unwrap().clone();
    let vad_threshold = state.settings.lock().unwrap().vad_threshold;

    let chunks = if !vad_path.is_empty() && std::path::Path::new(&vad_path).exists() {
        filetranscribe::vad_chunk(&samples, &vad_path, vad_threshold)?
    } else {
        vec![(0u64, samples.clone())]
    };

    let _ = app.emit(
        "file-transcribe-progress",
        json!({ "phase": "analyzing", "percent": 20, "filename": &filename }),
    );

    // Phase 3: Transcribe each chunk
    let engine_guard = state.engine.lock().unwrap();
    let engine_ref = engine_guard
        .as_ref()
        .ok_or("No speech engine loaded. Download a model first.")?;

    let total_chunks = chunks.len();
    let mut segments: Vec<serde_json::Value> = Vec::new();
    let mut full_text = String::new();

    for (i, (start_ms, chunk)) in chunks.iter().enumerate() {
        let pct = 20 + ((i as f32 / total_chunks as f32) * 80.0) as u32;
        let _ = app.emit(
            "file-transcribe-progress",
            json!({
                "phase": "transcribing", "percent": pct, "chunk": i + 1,
                "total_chunks": total_chunks, "filename": &filename
            }),
        );

        match engine_ref.transcribe(chunk) {
            Ok(text) if !text.is_empty() => {
                let end_ms = start_ms + (chunk.len() as u64 * 1000 / 16000);
                segments.push(json!({
                    "start_ms": start_ms,
                    "end_ms": end_ms,
                    "text": &text,
                }));
                if !full_text.is_empty() {
                    full_text.push(' ');
                }
                full_text.push_str(&text);
            }
            Ok(_) => {}
            Err(e) => log::warn!("Chunk {} transcription failed: {}", i, e),
        }
    }
    drop(engine_guard);

    let _ = app.emit(
        "file-transcribe-progress",
        json!({ "phase": "complete", "percent": 100, "filename": &filename }),
    );

    // Apply style formatting to full text
    let current_style = state.style.lock().unwrap().clone();
    let styled = current_style.format(&full_text);

    // Apply dictionary corrections
    let dict = state.dict.lock().unwrap();
    let styled = dict.apply(&styled);

    // Save to transcript history
    let model_name = state.model_name.lock().unwrap().clone();
    let db_guard = state.db.lock().unwrap();
    if let Some(db) = db_guard.as_ref() {
        let duration_ms = (duration_s * 1000.0) as i64;
        let style_name = format!("{:?}", current_style).to_lowercase();
        let _ = db.insert(&styled, &full_text, &style_name, &model_name, duration_ms);
    }

    log::info!(
        "File transcription complete: {} ({:.1}s, {} segments, {} chars)",
        filename,
        duration_s,
        segments.len(),
        styled.len()
    );

    Ok(json!({
        "filename": filename,
        "duration_s": duration_s,
        "text": styled,
        "raw_text": full_text,
        "segments": segments,
    }))
}

/// Generic model downloader. Downloads files from HuggingFace to the models directory.
#[tauri::command]
pub async fn download_model(app: tauri::AppHandle, model_id: String) -> Result<(), String> {
    use futures_util::StreamExt;
    use std::io::Write;

    let spec = models::find(&model_id).ok_or_else(|| format!("Unknown model: {}", model_id))?;
    let dir_name = spec.dir;
    let files = spec.files;
    let hf_base = spec.hf_base;

    let models_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("App data dir error: {}", e))?
        .join("models")
        .join(dir_name);

    std::fs::create_dir_all(&models_dir)
        .map_err(|e| format!("Failed to create model dir: {}", e))?;

    let total_bytes: u64 = spec.total_bytes();
    let mut downloaded: u64 = 0;
    let client = reqwest::Client::new();

    for (filename, _) in files {
        let dest = models_dir.join(filename);
        if dest.exists() {
            let file_size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
            downloaded += file_size;
            let pct = (downloaded * 100 / total_bytes).min(99) as u32;
            let _ = app.emit(
                "model-download-progress",
                serde_json::json!({
                    "percent": pct, "file": filename, "model": &model_id
                }),
            );
            continue;
        }

        let url = format!("{}/{}", hf_base, filename);
        log::info!("Downloading {} -> {}", url, dest.display());

        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Download failed {}: {}", filename, e))?;

        if !resp.status().is_success() {
            return Err(format!("HTTP {} for {}", resp.status(), filename));
        }

        let mut file =
            std::fs::File::create(&dest).map_err(|e| format!("Cannot create {}: {}", filename, e))?;

        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("Stream error: {}", e))?;
            file.write_all(&chunk)
                .map_err(|e| format!("Write error: {}", e))?;
            downloaded += chunk.len() as u64;
            let pct = (downloaded * 100 / total_bytes).min(99) as u32;
            let _ = app.emit(
                "model-download-progress",
                serde_json::json!({
                    "percent": pct, "file": filename, "model": &model_id
                }),
            );
        }

        log::info!("Downloaded {}", filename);
    }

    let _ = app.emit(
        "model-download-progress",
        serde_json::json!({ "percent": 100, "file": "done", "model": &model_id }),
    );
    log::info!("Model {} download complete", model_id);
    Ok(())
}

/// Remove a downloaded model's files from disk.
#[tauri::command]
pub fn remove_model(state: tauri::State<AppState>, model_id: String) -> Result<(), String> {
    let models_dir = state.models_dir.lock().unwrap().clone();
    let models_path = std::path::Path::new(&models_dir);

    let spec = models::find(&model_id).ok_or_else(|| format!("Unknown model: {}", model_id))?;
    let dir_name = spec.dir;

    // Don't allow removing the currently active model. model_name holds the
    // spec's display string, so this is a direct comparison rather than the
    // per-family string rebuilding it used to do.
    let current = state.model_name.lock().unwrap().clone();
    if current == spec.display {
        return Err(
            "Cannot remove the currently active model. Switch to another model first.".to_string(),
        );
    }

    let target = models_path.join(dir_name);
    if target.exists() {
        std::fs::remove_dir_all(&target)
            .map_err(|e| format!("Failed to remove {}: {}", dir_name, e))?;
        log::info!("Removed model: {} ({})", model_id, target.display());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Snippets commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_snippets(state: tauri::State<AppState>) -> Vec<snippets::Snippet> {
    state.snippet_store.lock().unwrap().snippets.clone()
}

#[tauri::command]
pub fn save_snippets(
    state: tauri::State<AppState>,
    items: Vec<snippets::Snippet>,
) -> Result<(), String> {
    let mut store = state.snippet_store.lock().unwrap();
    store.snippets = items;
    let path = state.snippets_path.lock().unwrap().clone();
    store.save(std::path::Path::new(&path))?;
    log::info!("Snippets saved: {} items", store.snippets.len());
    Ok(())
}

#[tauri::command]
pub fn test_snippet_expansion(state: tauri::State<AppState>, text: String) -> String {
    let store = state.snippet_store.lock().unwrap();
    store.expand(&text)
}

// ---------------------------------------------------------------------------
// Per-app style commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_app_styles(state: tauri::State<AppState>) -> appdetect::AppStyleRules {
    state.app_styles.lock().unwrap().clone()
}

#[tauri::command]
pub fn save_app_styles(
    state: tauri::State<AppState>,
    rules: appdetect::AppStyleRules,
) -> Result<(), String> {
    let path = state.app_styles_path.lock().unwrap().clone();
    rules.save(std::path::Path::new(&path))?;
    *state.app_styles.lock().unwrap() = rules;
    log::info!("App style rules saved");
    Ok(())
}

// ---------------------------------------------------------------------------
// Voice command commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_voice_commands(state: tauri::State<AppState>) -> voicecommand::VoiceCommandStore {
    state.voice_commands.lock().unwrap().clone()
}

#[tauri::command]
pub fn save_voice_commands(
    state: tauri::State<AppState>,
    store: voicecommand::VoiceCommandStore,
) -> Result<(), String> {
    let path = state.voice_commands_path.lock().unwrap().clone();
    store.save(std::path::Path::new(&path))?;
    *state.voice_commands.lock().unwrap() = store;
    log::info!("Voice commands saved");
    Ok(())
}
