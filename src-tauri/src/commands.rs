use crate::AppState;
use crate::{
    appdetect, dictionary, engine, export, filetranscribe, history, settings, snippets,
    style, voicecommand,
};
use tauri::{Emitter, Manager};

#[tauri::command]
pub fn get_model_name(state: tauri::State<AppState>) -> String {
    state.model_name.lock().unwrap().clone()
}

#[tauri::command]
pub fn get_installed_models(state: tauri::State<AppState>) -> serde_json::Value {
    let models_dir = state.models_dir.lock().unwrap().clone();
    let models_path = std::path::Path::new(&models_dir);

    let check = |dir: &str, files: &[&str]| -> bool {
        let d = models_path.join(dir);
        files.iter().any(|f| d.join(f).exists())
    };

    let encoder_files = &[
        "encoder.int8.onnx",
        "encoder.onnx",
        "encode.int8.onnx",
        "encode.onnx",
    ];

    serde_json::json!({
        "moonshine-tiny": check("moonshine-tiny", encoder_files),
        "moonshine-base": check("moonshine-base", encoder_files),
        "parakeet": check("parakeet-v3", encoder_files),
        "parakeet-v2": check("parakeet-v2", encoder_files),
        "whisper-tiny": check("whisper-tiny", &["tiny-encoder.int8.onnx", "tiny-encoder.onnx"]),
        "whisper-base": check("whisper-base", &["base-encoder.int8.onnx", "base-encoder.onnx"]),
        "whisper-small": check("whisper-small", &["small-encoder.int8.onnx", "small-encoder.onnx"]),
        "whisper-medium": check("whisper-medium", &["medium-encoder.int8.onnx", "medium-encoder.onnx"]),
        "whisper-large-v3": check("whisper-large-v3", &["large-v3-encoder.int8.onnx", "large-v3-encoder.onnx"]),
        "whisper-turbo": check("whisper-turbo", &["turbo-encoder.int8.onnx", "turbo-encoder.onnx"]),
        "whisper-distil-small-en": check("whisper-distil-small-en", &["distil-small.en-encoder.int8.onnx", "distil-small.en-encoder.onnx"]),
        "whisper-distil-medium-en": check("whisper-distil-medium-en", &["distil-medium.en-encoder.int8.onnx", "distil-medium.en-encoder.onnx"]),
        "sense-voice": check("sense-voice", &["model.int8.onnx", "model.onnx"]),
    })
}

#[tauri::command]
pub fn switch_model(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    model: String,
) -> Result<String, String> {
    let models_dir = state.models_dir.lock().unwrap().clone();
    let models_path = std::path::Path::new(&models_dir);

    let new_engine = match model.as_str() {
        "parakeet" => engine::SpeechEngine::parakeet(models_path)?,
        "parakeet-v2" => engine::SpeechEngine::parakeet_v2(models_path)?,
        "moonshine-tiny" => engine::SpeechEngine::moonshine(models_path, "tiny")?,
        "moonshine-base" => engine::SpeechEngine::moonshine(models_path, "base")?,
        "whisper-tiny" => engine::SpeechEngine::whisper(models_path, "tiny")?,
        "whisper-base" => engine::SpeechEngine::whisper(models_path, "base")?,
        "whisper-small" => engine::SpeechEngine::whisper(models_path, "small")?,
        "whisper-medium" => engine::SpeechEngine::whisper(models_path, "medium")?,
        "whisper-large-v3" => engine::SpeechEngine::whisper(models_path, "large-v3")?,
        "whisper-turbo" => engine::SpeechEngine::whisper(models_path, "turbo")?,
        "whisper-distil-small-en" => engine::SpeechEngine::whisper(models_path, "distil-small.en")?,
        "whisper-distil-medium-en" => {
            engine::SpeechEngine::whisper(models_path, "distil-medium.en")?
        }
        "sense-voice" => engine::SpeechEngine::sense_voice(models_path)?,
        _ => return Err(format!("Unknown model: {}", model)),
    };

    let name_str = match new_engine.model_type() {
        engine::ModelType::MoonshineTiny => "Moonshine Tiny".to_string(),
        engine::ModelType::MoonshineBase => "Moonshine Base".to_string(),
        engine::ModelType::MoonshineMedium => "Moonshine Medium".to_string(),
        engine::ModelType::Parakeet => "Parakeet V3".to_string(),
        engine::ModelType::ParakeetV2 => "Parakeet V2".to_string(),
        engine::ModelType::Whisper(n) => format!("Whisper {}", n),
        engine::ModelType::SenseVoice => "SenseVoice".to_string(),
        engine::ModelType::CanaryFlash => "Canary Flash".to_string(),
    };

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
    state: tauri::State<AppState>,
    key: String,
    value: String,
) -> Result<(), String> {
    let mut settings = state.settings.lock().unwrap();
    match key.as_str() {
        "style" => settings.style = value,
        "model" => settings.model = value,
        "hotkey" => settings.hotkey = value,
        "recording_mode" => settings.recording_mode = value,
        "start_on_boot" => settings.start_on_boot = value == "true",
        "show_overlay" => settings.show_overlay = value == "true",
        "advanced_mode" => settings.advanced_mode = value == "true",
        "mic_device" => settings.mic_device = value,
        "vad_threshold" => settings.vad_threshold = value.parse().unwrap_or(0.5),
        "sound_dictation" => {
            settings.sound_dictation = value == "true";
            crate::sounds::set_dictation_sounds(settings.sound_dictation);
        }
        "sound_agent" => {
            settings.sound_agent = value == "true";
            crate::sounds::set_agent_sounds(settings.sound_agent);
        }
        _ => return Err(format!("Unknown setting: {}", key)),
    }
    let path = state.settings_path.lock().unwrap().clone();
    settings.save(std::path::Path::new(&path))
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

#[tauri::command]
pub async fn download_parakeet(app: tauri::AppHandle) -> Result<(), String> {
    use futures_util::StreamExt;
    use std::io::Write;

    let models_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("App data dir error: {}", e))?
        .join("models")
        .join("parakeet-v3");

    std::fs::create_dir_all(&models_dir)
        .map_err(|e| format!("Failed to create model dir: {}", e))?;

    let base = "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main";
    let files = vec![
        ("encoder.int8.onnx", 683_000_000u64),
        ("decoder.int8.onnx", 12_000_000u64),
        ("joiner.int8.onnx", 7_000_000u64),
        ("tokens.txt", 96_000u64),
    ];

    let total_bytes: u64 = files.iter().map(|(_, s)| s).sum();
    let mut downloaded: u64 = 0;

    let client = reqwest::Client::new();

    for (filename, _expected) in &files {
        let dest = models_dir.join(filename);
        if dest.exists() {
            let file_size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
            downloaded += file_size;
            let pct = (downloaded * 100 / total_bytes) as u32;
            let _ = app.emit(
                "model-download-progress",
                serde_json::json!({ "percent": pct, "file": filename }),
            );
            continue;
        }

        let url = format!("{}/{}", base, filename);
        log::info!("Downloading {}", url);

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
                serde_json::json!({ "percent": pct, "file": filename }),
            );
        }

        log::info!("Downloaded {}", filename);
    }

    let _ = app.emit(
        "model-download-progress",
        serde_json::json!({ "percent": 100, "file": "done" }),
    );
    log::info!("Parakeet V3 download complete");
    Ok(())
}

/// Generic model downloader. Downloads files from HuggingFace to the models directory.
#[tauri::command]
pub async fn download_model(app: tauri::AppHandle, model_id: String) -> Result<(), String> {
    use futures_util::StreamExt;
    use std::io::Write;

    let (dir_name, files): (&str, Vec<(&str, u64)>) = match model_id.as_str() {
        "parakeet" => (
            "parakeet-v3",
            vec![
                ("encoder.int8.onnx", 683_000_000),
                ("decoder.int8.onnx", 12_000_000),
                ("joiner.int8.onnx", 7_000_000),
                ("tokens.txt", 96_000),
            ],
        ),
        "parakeet-v2" => (
            "parakeet-v2",
            vec![
                ("encoder.int8.onnx", 683_000_000),
                ("decoder.int8.onnx", 12_000_000),
                ("joiner.int8.onnx", 7_000_000),
                ("tokens.txt", 96_000),
            ],
        ),
        "moonshine-base" => (
            "moonshine-base",
            vec![
                ("preprocess.onnx", 14_100_000),
                ("encode.int8.onnx", 50_300_000),
                ("cached_decode.int8.onnx", 100_000_000),
                ("uncached_decode.int8.onnx", 122_000_000),
                ("tokens.txt", 437_000),
            ],
        ),
        "whisper-tiny" => (
            "whisper-tiny",
            vec![
                ("tiny-encoder.int8.onnx", 12_000_000),
                ("tiny-decoder.int8.onnx", 86_000_000),
                ("tiny-tokens.txt", 800_000),
            ],
        ),
        "whisper-base" => (
            "whisper-base",
            vec![
                ("base-encoder.int8.onnx", 20_000_000),
                ("base-decoder.int8.onnx", 114_000_000),
                ("base-tokens.txt", 800_000),
            ],
        ),
        "whisper-small" => (
            "whisper-small",
            vec![
                ("small-encoder.int8.onnx", 72_000_000),
                ("small-decoder.int8.onnx", 305_000_000),
                ("small-tokens.txt", 800_000),
            ],
        ),
        "whisper-medium" => (
            "whisper-medium",
            vec![
                ("medium-encoder.int8.onnx", 193_000_000),
                ("medium-decoder.int8.onnx", 823_000_000),
                ("medium-tokens.txt", 800_000),
            ],
        ),
        "whisper-large-v3" => (
            "whisper-large-v3",
            vec![
                ("large-v3-encoder.int8.onnx", 397_000_000),
                ("large-v3-decoder.int8.onnx", 1_100_000_000),
                ("large-v3-tokens.txt", 800_000),
            ],
        ),
        "whisper-turbo" => (
            "whisper-turbo",
            vec![
                ("turbo-encoder.int8.onnx", 397_000_000),
                ("turbo-decoder.int8.onnx", 409_000_000),
                ("turbo-tokens.txt", 800_000),
            ],
        ),
        "whisper-distil-small-en" => (
            "whisper-distil-small-en",
            vec![
                ("distil-small.en-encoder.int8.onnx", 72_000_000),
                ("distil-small.en-decoder.int8.onnx", 108_000_000),
                ("distil-small.en-tokens.txt", 800_000),
            ],
        ),
        "whisper-distil-medium-en" => (
            "whisper-distil-medium-en",
            vec![
                ("distil-medium.en-encoder.int8.onnx", 193_000_000),
                ("distil-medium.en-decoder.int8.onnx", 270_000_000),
                ("distil-medium.en-tokens.txt", 800_000),
            ],
        ),
        "sense-voice" => (
            "sense-voice",
            vec![
                ("model.int8.onnx", 160_000_000),
                ("tokens.txt", 50_000),
            ],
        ),
        _ => return Err(format!("Unknown model: {}", model_id)),
    };

    let hf_base = match model_id.as_str() {
        "parakeet" => "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main",
        "parakeet-v2" => "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/resolve/main",
        "moonshine-base" => "https://huggingface.co/csukuangfj/sherpa-onnx-moonshine-base-en-int8/resolve/main",
        "whisper-tiny" => "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-tiny/resolve/main",
        "whisper-base" => "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-base/resolve/main",
        "whisper-small" => "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-small/resolve/main",
        "whisper-medium" => "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-medium/resolve/main",
        "whisper-large-v3" => "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-large-v3/resolve/main",
        "whisper-turbo" => "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-turbo/resolve/main",
        "whisper-distil-small-en" => "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-distil-small.en/resolve/main",
        "whisper-distil-medium-en" => "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-distil-medium.en/resolve/main",
        "sense-voice" => "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main",
        _ => return Err(format!("No download URL for: {}", model_id)),
    };

    let models_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("App data dir error: {}", e))?
        .join("models")
        .join(dir_name);

    std::fs::create_dir_all(&models_dir)
        .map_err(|e| format!("Failed to create model dir: {}", e))?;

    let total_bytes: u64 = files.iter().map(|(_, s)| s).sum();
    let mut downloaded: u64 = 0;
    let client = reqwest::Client::new();

    for (filename, _) in &files {
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

    let dir_name = match model_id.as_str() {
        "parakeet" => "parakeet-v3",
        "parakeet-v2" => "parakeet-v2",
        "moonshine-tiny" => "moonshine-tiny",
        "moonshine-base" => "moonshine-base",
        "whisper-tiny" => "whisper-tiny",
        "whisper-base" => "whisper-base",
        "whisper-small" => "whisper-small",
        "whisper-medium" => "whisper-medium",
        "whisper-large-v3" => "whisper-large-v3",
        "whisper-turbo" => "whisper-turbo",
        "whisper-distil-small-en" => "whisper-distil-small-en",
        "whisper-distil-medium-en" => "whisper-distil-medium-en",
        "sense-voice" => "sense-voice",
        _ => return Err(format!("Unknown model: {}", model_id)),
    };

    // Don't allow removing the currently active model
    let current = state.model_name.lock().unwrap().clone();
    let _is_active = match model_id.as_str() {
        "parakeet" => current == "Parakeet V3",
        "moonshine-tiny" => current == "Moonshine Tiny",
        "sense-voice" => current == "SenseVoice",
        id if id.starts_with("whisper-") => current.to_lowercase().contains("whisper"),
        _ => false,
    };
    // More precise active check for whisper variants
    let is_active = match model_id.as_str() {
        "parakeet" => current == "Parakeet V3",
        "parakeet-v2" => current == "Parakeet V2",
        "moonshine-tiny" => current == "Moonshine Tiny",
        "moonshine-base" => current == "Moonshine Base",
        "sense-voice" => current == "SenseVoice",
        _ => {
            let variant_name = dir_name.replace("whisper-", "");
            current == format!("Whisper {}", variant_name)
        }
    };

    if is_active {
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

// ---------------------------------------------------------------------------
// Agent (OpenClaw) commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn save_agent_token(state: tauri::State<AppState>, token: String) -> Result<(), String> {
    // Try keyring first, fall back to settings file
    let keyring_ok = keyring::Entry::new("inkwell", "openclaw")
        .ok()
        .and_then(|e| e.set_password(&token).ok())
        .is_some();

    if keyring_ok {
        log::info!("Agent: OpenClaw token saved to keyring");
    } else {
        log::warn!("Agent: keyring failed, saving token to settings");
    }

    // Always save to settings as backup
    let mut settings = state.settings.lock().unwrap();
    settings.agent_token = token;
    let path = state.settings_path.lock().unwrap().clone();
    let _ = settings.save(std::path::Path::new(&path));
    log::info!("Agent: token saved to settings");
    Ok(())
}

#[tauri::command]
pub fn get_agent_token_status(state: tauri::State<AppState>) -> bool {
    // Check keyring first, then settings
    let from_keyring = keyring::Entry::new("inkwell", "openclaw")
        .ok()
        .and_then(|e| e.get_password().ok())
        .map(|k| !k.is_empty())
        .unwrap_or(false);
    if from_keyring { return true; }

    let settings = state.settings.lock().unwrap();
    !settings.agent_token.is_empty()
}

#[tauri::command]
pub async fn test_agent_connection(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let (url, settings_token) = {
        let settings = state.settings.lock().unwrap();
        (settings.agent_url.clone(), settings.agent_token.clone())
    };

    let token = keyring::Entry::new("inkwell", "openclaw")
        .ok()
        .and_then(|e| e.get_password().ok())
        .filter(|k| !k.is_empty())
        .unwrap_or(settings_token);

    if token.is_empty() {
        return Err("No token configured".to_string());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("Client error: {}", e))?;

    let resp = client
        .get(format!("{}/health", url))
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("Connection failed: {}", e))?;

    if resp.status().is_success() {
        Ok("Connected to OpenClaw".to_string())
    } else {
        Err(format!("OpenClaw returned status {}", resp.status()))
    }
}
