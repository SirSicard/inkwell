mod appdetect;
mod audio;
mod dictionary;
mod engine;
mod export;
mod filetranscribe;
mod history;
mod llm;
mod overlay;
mod paste;
mod recording;
mod settings;
mod snippets;
mod style;
mod usage;
mod vad;
mod voicecommand;

use audio::AudioState;
use engine::SpeechEngine;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use tauri::{Emitter, Manager};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;

struct AppState {
    audio: Mutex<Option<AudioState>>,
    engine: Mutex<Option<SpeechEngine>>,
    models_dir: Mutex<String>,
    vad_model_path: Mutex<String>,
    model_name: Mutex<String>,
    style: Mutex<style::Style>,
    settings: Mutex<settings::Settings>,
    settings_path: Mutex<String>,
    db: Mutex<Option<history::TranscriptDb>>,
    dict: Mutex<dictionary::Dictionary>,
    dict_path: Mutex<String>,
    is_first_run: Mutex<bool>,
    // AI Polish
    polish_enabled: Mutex<bool>,
    polish_prompt: Mutex<String>,
    usage: Mutex<usage::UsageData>,
    usage_path: Mutex<String>,
    // Snippets
    snippet_store: Mutex<snippets::SnippetStore>,
    snippets_path: Mutex<String>,
    // Per-app styles
    app_styles: Mutex<appdetect::AppStyleRules>,
    app_styles_path: Mutex<String>,
    // Voice commands
    voice_commands: Mutex<voicecommand::VoiceCommandStore>,
    voice_commands_path: Mutex<String>,
}

#[tauri::command]
fn get_model_name(state: tauri::State<AppState>) -> String {
    state.model_name.lock().unwrap().clone()
}

#[tauri::command]
fn get_installed_models(state: tauri::State<AppState>) -> serde_json::Value {
    let models_dir = state.models_dir.lock().unwrap().clone();
    let models_path = std::path::Path::new(&models_dir);

    // Check each model directory for its key file
    let check = |dir: &str, files: &[&str]| -> bool {
        let d = models_path.join(dir);
        files.iter().any(|f| d.join(f).exists())
    };

    let encoder_files = &["encoder.int8.onnx", "encoder.onnx", "encode.int8.onnx", "encode.onnx"];

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
fn switch_model(app: tauri::AppHandle, state: tauri::State<AppState>, model: String) -> Result<String, String> {
    let models_dir = state.models_dir.lock().unwrap().clone();
    let models_path = std::path::Path::new(&models_dir);

    let engine = match model.as_str() {
        "parakeet" => SpeechEngine::parakeet(models_path)?,
        "parakeet-v2" => SpeechEngine::parakeet_v2(models_path)?,
        "moonshine-tiny" => SpeechEngine::moonshine(models_path, "tiny")?,
        "moonshine-base" => SpeechEngine::moonshine(models_path, "base")?,
        "whisper-tiny" => SpeechEngine::whisper(models_path, "tiny")?,
        "whisper-base" => SpeechEngine::whisper(models_path, "base")?,
        "whisper-small" => SpeechEngine::whisper(models_path, "small")?,
        "whisper-medium" => SpeechEngine::whisper(models_path, "medium")?,
        "whisper-large-v3" => SpeechEngine::whisper(models_path, "large-v3")?,
        "whisper-turbo" => SpeechEngine::whisper(models_path, "turbo")?,
        "whisper-distil-small-en" => SpeechEngine::whisper(models_path, "distil-small.en")?,
        "whisper-distil-medium-en" => SpeechEngine::whisper(models_path, "distil-medium.en")?,
        "sense-voice" => SpeechEngine::sense_voice(models_path)?,
        _ => return Err(format!("Unknown model: {}", model)),
    };

    let name_str = match engine.model_type() {
        engine::ModelType::MoonshineTiny => "Moonshine Tiny".to_string(),
        engine::ModelType::MoonshineBase => "Moonshine Base".to_string(),
        engine::ModelType::MoonshineMedium => "Moonshine Medium".to_string(),
        engine::ModelType::Parakeet => "Parakeet V3".to_string(),
        engine::ModelType::ParakeetV2 => "Parakeet V2".to_string(),
        engine::ModelType::Whisper(n) => format!("Whisper {}", n),
        engine::ModelType::SenseVoice => "SenseVoice".to_string(),
    };

    *state.model_name.lock().unwrap() = name_str.clone();
    *state.engine.lock().unwrap() = Some(engine);
    let _ = app.emit("model-loaded", &name_str);
    log::info!("Switched to model: {}", name_str);

    // Persist
    let mut settings = state.settings.lock().unwrap();
    settings.model = model;
    let path = state.settings_path.lock().unwrap().clone();
    let _ = settings.save(std::path::Path::new(&path));

    Ok(name_str)
}

#[tauri::command]
fn set_style(state: tauri::State<AppState>, style_name: String) -> Result<(), String> {
    let s: style::Style = serde_json::from_str(&format!("\"{}\"", style_name))
        .map_err(|_| format!("Unknown style: {}", style_name))?;
    log::info!("Style set to: {:?}", s);
    *state.style.lock().unwrap() = s;

    // Persist
    let mut settings = state.settings.lock().unwrap();
    settings.style = style_name;
    let path = state.settings_path.lock().unwrap().clone();
    let _ = settings.save(std::path::Path::new(&path));
    Ok(())
}

#[tauri::command]
fn get_settings(state: tauri::State<AppState>) -> settings::Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn update_settings(state: tauri::State<AppState>, key: String, value: String) -> Result<(), String> {
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
        _ => return Err(format!("Unknown setting: {}", key)),
    }
    let path = state.settings_path.lock().unwrap().clone();
    settings.save(std::path::Path::new(&path))
}

#[tauri::command]
fn get_transcripts(state: tauri::State<AppState>, limit: Option<usize>) -> Result<Vec<history::Transcript>, String> {
    let db_guard = state.db.lock().unwrap();
    match db_guard.as_ref() {
        Some(db) => db.recent(limit.unwrap_or(50)),
        None => Ok(vec![]),
    }
}

#[tauri::command]
fn search_transcripts(state: tauri::State<AppState>, query: String, limit: Option<usize>) -> Result<Vec<history::Transcript>, String> {
    let db_guard = state.db.lock().unwrap();
    match db_guard.as_ref() {
        Some(db) => db.search(&query, limit.unwrap_or(50)),
        None => Ok(vec![]),
    }
}

#[tauri::command]
fn delete_transcript(state: tauri::State<AppState>, id: i64) -> Result<(), String> {
    let db_guard = state.db.lock().unwrap();
    match db_guard.as_ref() {
        Some(db) => db.delete(id),
        None => Err("No database".to_string()),
    }
}

#[tauri::command]
fn check_first_run(state: tauri::State<AppState>) -> bool {
    let mut first = state.is_first_run.lock().unwrap();
    let val = *first;
    *first = false; // Only report once
    val
}

#[tauri::command]
fn set_hotkey(app: tauri::AppHandle, state: tauri::State<AppState>, hotkey: String) -> Result<(), String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;

    // Parse the new shortcut
    let shortcut: tauri_plugin_global_shortcut::Shortcut = hotkey.parse()
        .map_err(|e| format!("Invalid hotkey '{}': {}", hotkey, e))?;

    // Unregister all existing shortcuts
    let manager = app.global_shortcut();
    let _ = manager.unregister_all();

    // Register the new one
    manager.register(shortcut)
        .map_err(|e| format!("Failed to register '{}': {}", hotkey, e))?;

    // Persist
    let mut settings = state.settings.lock().unwrap();
    settings.hotkey = hotkey.clone();
    let path = state.settings_path.lock().unwrap().clone();
    let _ = settings.save(std::path::Path::new(&path));

    log::info!("Hotkey changed to: {}", hotkey);
    Ok(())
}

#[tauri::command]
fn get_vad_threshold(state: tauri::State<AppState>) -> f32 {
    state.settings.lock().unwrap().vad_threshold
}

#[tauri::command]
fn set_vad_threshold(state: tauri::State<AppState>, threshold: f32) -> Result<(), String> {
    let mut settings = state.settings.lock().unwrap();
    settings.vad_threshold = threshold.clamp(0.1, 0.95);
    let path = state.settings_path.lock().unwrap().clone();
    settings.save(std::path::Path::new(&path))?;
    log::info!("VAD threshold set to: {:.2}", settings.vad_threshold);
    Ok(())
}

#[tauri::command]
fn get_dictionary(state: tauri::State<AppState>) -> Vec<dictionary::DictEntry> {
    state.dict.lock().unwrap().entries.clone()
}

#[tauri::command]
fn export_transcripts(
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
        "txt"  => export::to_txt(&transcripts),
        "srt"  => export::to_srt(&transcripts),
        "json" => export::to_json(&transcripts),
        "csv"  => export::to_csv(&transcripts),
        other  => return Err(format!("Unknown format: {}", other)),
    };

    log::info!("Exported {} transcripts as {}", transcripts.len(), format);
    Ok(content)
}

/// Transcribe an audio/video file. Decodes, runs VAD, chunks, and transcribes.
/// Emits progress events: file-transcribe-progress { phase, percent, text? }
#[tauri::command]
async fn transcribe_file(
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

    let filename = file_path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    // Phase 1: Decode
    let _ = app.emit("file-transcribe-progress", json!({
        "phase": "decoding", "percent": 5, "filename": &filename
    }));
    let samples = filetranscribe::decode_to_pcm(file_path)?;
    let duration_s = samples.len() as f32 / 16000.0;

    let _ = app.emit("file-transcribe-progress", json!({
        "phase": "decoding", "percent": 15, "filename": &filename
    }));

    // Phase 2: VAD chunking
    let _ = app.emit("file-transcribe-progress", json!({
        "phase": "analyzing", "percent": 18, "filename": &filename
    }));
    let vad_path = state.vad_model_path.lock().unwrap().clone();
    let vad_threshold = state.settings.lock().unwrap().vad_threshold;

    let chunks = if !vad_path.is_empty() && std::path::Path::new(&vad_path).exists() {
        filetranscribe::vad_chunk(&samples, &vad_path, vad_threshold)?
    } else {
        // No VAD: single chunk
        vec![(0u64, samples.clone())]
    };

    let _ = app.emit("file-transcribe-progress", json!({
        "phase": "analyzing", "percent": 20, "filename": &filename
    }));

    // Phase 3: Transcribe each chunk
    let engine_guard = state.engine.lock().unwrap();
    let engine = engine_guard.as_ref()
        .ok_or("No speech engine loaded. Download a model first.")?;

    let total_chunks = chunks.len();
    let mut segments: Vec<serde_json::Value> = Vec::new();
    let mut full_text = String::new();

    for (i, (start_ms, chunk)) in chunks.iter().enumerate() {
        let pct = 20 + ((i as f32 / total_chunks as f32) * 80.0) as u32;
        let _ = app.emit("file-transcribe-progress", json!({
            "phase": "transcribing", "percent": pct, "chunk": i + 1,
            "total_chunks": total_chunks, "filename": &filename
        }));

        match engine.transcribe(chunk) {
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
            Ok(_) => {} // empty transcription, skip
            Err(e) => log::warn!("Chunk {} transcription failed: {}", i, e),
        }
    }
    drop(engine_guard);

    let _ = app.emit("file-transcribe-progress", json!({
        "phase": "complete", "percent": 100, "filename": &filename
    }));

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
        filename, duration_s, segments.len(), styled.len()
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
async fn download_parakeet(app: tauri::AppHandle) -> Result<(), String> {
    use futures_util::StreamExt;
    use std::io::Write;

    let models_dir = app.path().app_data_dir()
        .map_err(|e| format!("App data dir error: {}", e))?
        .join("models")
        .join("parakeet-v3");

    std::fs::create_dir_all(&models_dir)
        .map_err(|e| format!("Failed to create model dir: {}", e))?;

    let base = "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main";
    let files = vec![
        ("encoder.int8.onnx", 683_000_000u64),
        ("decoder.int8.onnx", 12_000_000u64),
        ("joiner.int8.onnx",   7_000_000u64),
        ("tokens.txt",            96_000u64),
    ];

    let total_bytes: u64 = files.iter().map(|(_, s)| s).sum();
    let mut downloaded: u64 = 0;

    let client = reqwest::Client::new();

    for (filename, _expected) in &files {
        let dest = models_dir.join(filename);
        if dest.exists() {
            // Skip if already present
            let file_size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
            downloaded += file_size;
            let pct = (downloaded * 100 / total_bytes) as u32;
            let _ = app.emit("model-download-progress", serde_json::json!({ "percent": pct, "file": filename }));
            continue;
        }

        let url = format!("{}/{}", base, filename);
        log::info!("Downloading {}", url);

        let resp = client.get(&url)
            .send()
            .await
            .map_err(|e| format!("Download failed {}: {}", filename, e))?;

        if !resp.status().is_success() {
            return Err(format!("HTTP {} for {}", resp.status(), filename));
        }

        let mut file = std::fs::File::create(&dest)
            .map_err(|e| format!("Cannot create {}: {}", filename, e))?;

        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("Stream error: {}", e))?;
            file.write_all(&chunk).map_err(|e| format!("Write error: {}", e))?;
            downloaded += chunk.len() as u64;
            let pct = (downloaded * 100 / total_bytes).min(99) as u32;
            let _ = app.emit("model-download-progress", serde_json::json!({ "percent": pct, "file": filename }));
        }

        log::info!("Downloaded {}", filename);
    }

    let _ = app.emit("model-download-progress", serde_json::json!({ "percent": 100, "file": "done" }));
    log::info!("Parakeet V3 download complete");
    Ok(())
}

/// Generic model downloader. Downloads files from HuggingFace to the models directory.
/// model_id maps to a known set of (hf_repo, dir_name, files).
#[tauri::command]
async fn download_model(app: tauri::AppHandle, model_id: String) -> Result<(), String> {
    use futures_util::StreamExt;
    use std::io::Write;

    // Model registry: (dir_name, hf_base_url, files with estimated sizes)
    let (dir_name, files): (&str, Vec<(&str, u64)>) = match model_id.as_str() {
        "parakeet" => ("parakeet-v3", vec![
            ("encoder.int8.onnx", 683_000_000),
            ("decoder.int8.onnx", 12_000_000),
            ("joiner.int8.onnx", 7_000_000),
            ("tokens.txt", 96_000),
        ]),
        "parakeet-v2" => ("parakeet-v2", vec![
            ("encoder.int8.onnx", 683_000_000),
            ("decoder.int8.onnx", 12_000_000),
            ("joiner.int8.onnx", 7_000_000),
            ("tokens.txt", 96_000),
        ]),
        "moonshine-base" => ("moonshine-base", vec![
            ("preprocess.onnx", 14_100_000),
            ("encode.int8.onnx", 50_300_000),
            ("cached_decode.int8.onnx", 100_000_000),
            ("uncached_decode.int8.onnx", 122_000_000),
            ("tokens.txt", 437_000),
        ]),
        "whisper-tiny" => ("whisper-tiny", vec![
            ("tiny-encoder.int8.onnx", 12_000_000),
            ("tiny-decoder.int8.onnx", 86_000_000),
            ("tiny-tokens.txt", 800_000),
        ]),
        "whisper-base" => ("whisper-base", vec![
            ("base-encoder.int8.onnx", 20_000_000),
            ("base-decoder.int8.onnx", 114_000_000),
            ("base-tokens.txt", 800_000),
        ]),
        "whisper-small" => ("whisper-small", vec![
            ("small-encoder.int8.onnx", 72_000_000),
            ("small-decoder.int8.onnx", 305_000_000),
            ("small-tokens.txt", 800_000),
        ]),
        "whisper-medium" => ("whisper-medium", vec![
            ("medium-encoder.int8.onnx", 193_000_000),
            ("medium-decoder.int8.onnx", 823_000_000),
            ("medium-tokens.txt", 800_000),
        ]),
        "whisper-large-v3" => ("whisper-large-v3", vec![
            ("large-v3-encoder.int8.onnx", 397_000_000),
            ("large-v3-decoder.int8.onnx", 1_100_000_000),
            ("large-v3-tokens.txt", 800_000),
        ]),
        "whisper-turbo" => ("whisper-turbo", vec![
            ("turbo-encoder.int8.onnx", 397_000_000),
            ("turbo-decoder.int8.onnx", 409_000_000),
            ("turbo-tokens.txt", 800_000),
        ]),
        "whisper-distil-small-en" => ("whisper-distil-small-en", vec![
            ("distil-small.en-encoder.int8.onnx", 72_000_000),
            ("distil-small.en-decoder.int8.onnx", 108_000_000),
            ("distil-small.en-tokens.txt", 800_000),
        ]),
        "whisper-distil-medium-en" => ("whisper-distil-medium-en", vec![
            ("distil-medium.en-encoder.int8.onnx", 193_000_000),
            ("distil-medium.en-decoder.int8.onnx", 270_000_000),
            ("distil-medium.en-tokens.txt", 800_000),
        ]),
        "sense-voice" => ("sense-voice", vec![
            ("model.int8.onnx", 160_000_000),
            ("tokens.txt", 50_000),
        ]),
        _ => return Err(format!("Unknown model: {}", model_id)),
    };

    // HuggingFace repo URL mapping
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

    let models_dir = app.path().app_data_dir()
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
            let _ = app.emit("model-download-progress", serde_json::json!({
                "percent": pct, "file": filename, "model": &model_id
            }));
            continue;
        }

        let url = format!("{}/{}", hf_base, filename);
        log::info!("Downloading {} -> {}", url, dest.display());

        let resp = client.get(&url)
            .send()
            .await
            .map_err(|e| format!("Download failed {}: {}", filename, e))?;

        if !resp.status().is_success() {
            return Err(format!("HTTP {} for {}", resp.status(), filename));
        }

        let mut file = std::fs::File::create(&dest)
            .map_err(|e| format!("Cannot create {}: {}", filename, e))?;

        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("Stream error: {}", e))?;
            file.write_all(&chunk).map_err(|e| format!("Write error: {}", e))?;
            downloaded += chunk.len() as u64;
            let pct = (downloaded * 100 / total_bytes).min(99) as u32;
            let _ = app.emit("model-download-progress", serde_json::json!({
                "percent": pct, "file": filename, "model": &model_id
            }));
        }

        log::info!("Downloaded {}", filename);
    }

    let _ = app.emit("model-download-progress", serde_json::json!({
        "percent": 100, "file": "done", "model": &model_id
    }));
    log::info!("Model {} download complete", model_id);
    Ok(())
}

/// Remove a downloaded model's files from disk.
#[tauri::command]
fn remove_model(state: tauri::State<AppState>, model_id: String) -> Result<(), String> {
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
    let is_active = match model_id.as_str() {
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
            // For whisper models, check if the current model name matches this variant
            let variant_name = dir_name.replace("whisper-", "");
            current == format!("Whisper {}", variant_name)
        }
    };

    if is_active {
        return Err("Cannot remove the currently active model. Switch to another model first.".to_string());
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
// AI Polish / BYOK commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn save_api_key(provider: String, key: String) -> Result<(), String> {
    let entry = keyring::Entry::new("inkwell", &provider)
        .map_err(|e| format!("Keyring error: {}", e))?;
    if key.is_empty() {
        // Delete key
        let _ = entry.delete_credential();
    } else {
        entry.set_password(&key)
            .map_err(|e| format!("Failed to save key: {}", e))?;
    }
    log::info!("API key saved for provider: {}", provider);
    Ok(())
}

#[tauri::command]
fn get_api_key_status() -> serde_json::Value {
    let providers = ["openai", "groq", "anthropic", "openrouter", "custom"];
    let mut status = serde_json::Map::new();
    for p in &providers {
        let configured = keyring::Entry::new("inkwell", p)
            .ok()
            .and_then(|e| e.get_password().ok())
            .map(|k| !k.is_empty())
            .unwrap_or(false);
        status.insert(p.to_string(), serde_json::Value::Bool(configured));
    }
    serde_json::Value::Object(status)
}

#[tauri::command]
fn get_polish_settings(state: tauri::State<AppState>) -> serde_json::Value {
    let enabled = *state.polish_enabled.lock().unwrap();
    let prompt = state.polish_prompt.lock().unwrap().clone();
    serde_json::json!({ "enabled": enabled, "prompt": prompt })
}

#[tauri::command]
fn set_polish_settings(state: tauri::State<AppState>, enabled: bool, prompt: String) {
    *state.polish_enabled.lock().unwrap() = enabled;
    *state.polish_prompt.lock().unwrap() = prompt.clone();

    // Persist to settings.json
    let mut settings = state.settings.lock().unwrap();
    settings.polish_enabled = enabled;
    settings.polish_prompt = prompt;
    let path = state.settings_path.lock().unwrap().clone();
    let _ = settings.save(std::path::Path::new(&path));
    log::info!("Polish settings saved: enabled={}", enabled);
}

#[tauri::command]
fn get_usage(state: tauri::State<AppState>) -> serde_json::Value {
    let usage = state.usage.lock().unwrap();
    serde_json::json!({
        "words_used": usage.words_used,
        "free_tier": usage::FREE_TIER_WORDS,
        "remaining": usage.remaining(),
        "over_limit": usage.over_limit(),
        "week_start": usage.week_start,
    })
}

#[tauri::command]
async fn run_ai_polish(
    state: tauri::State<'_, AppState>,
    text: String,
    provider: Option<String>,  // None = use proxy (free tier)
    model: Option<String>,
) -> Result<serde_json::Value, String> {
    let prompt = state.polish_prompt.lock().unwrap().clone();
    let install_id = state.settings.lock().unwrap().install_id.clone();

    let provider = provider.unwrap_or_else(|| "proxy".to_string());

    if provider == "proxy" {
        // Free tier via Cloudflare Worker
        let result = llm::call_proxy(&install_id, &text, &prompt).await?;
        let polished = result.text.unwrap_or(text);

        // Update local usage tracking
        let mut usage = state.usage.lock().unwrap();
        usage.ensure_current();
        if let (Some(used), Some(limit)) = (result.words_used, result.limit) {
            // Sync from server-side count
            usage.words_used = used;
            let path = state.usage_path.lock().unwrap().clone();
            let _ = usage.save(std::path::Path::new(&path));
            let remaining = limit.saturating_sub(used);
            log::info!("AI Polish (proxy): {}/{} words used this week", used, limit);
            return Ok(serde_json::json!({ "text": polished, "words_used": used, "remaining": remaining }));
        } else {
            // Server didn't return usage; count locally
            usage.add_words(&polished);
            let path = state.usage_path.lock().unwrap().clone();
            let _ = usage.save(std::path::Path::new(&path));
            let words_used = usage.words_used;
            let remaining = usage.remaining();
            log::info!("AI Polish (proxy, local count): {}/{} words this week", words_used, usage::FREE_TIER_WORDS);
            return Ok(serde_json::json!({ "text": polished, "words_used": words_used, "remaining": remaining }));
        }
    }

    // BYOK path
    let api_key = keyring::Entry::new("inkwell", &provider)
        .map_err(|e| format!("Keyring error: {}", e))?
        .get_password()
        .map_err(|_| format!("No API key configured for {}. Add one in Settings → AI.", provider))?;

    if api_key.is_empty() {
        return Err(format!("No API key configured for {}. Add one in Settings → AI.", provider));
    }

    let cfg = llm::ProviderConfig {
        provider: provider.clone(),
        api_key,
        custom_url: None,
        model,
    };

    let llm = llm::build_provider(cfg);
    let result = llm.complete(&prompt, &text).await?;

    log::info!("AI Polish (BYOK {}) {} chars -> {} chars", provider, text.len(), result.text.len());
    Ok(serde_json::json!({ "text": result.text }))
}

// ---------------------------------------------------------------------------
// Per-app style commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_app_styles(state: tauri::State<AppState>) -> appdetect::AppStyleRules {
    state.app_styles.lock().unwrap().clone()
}

#[tauri::command]
fn save_app_styles(state: tauri::State<AppState>, rules: appdetect::AppStyleRules) -> Result<(), String> {
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
fn get_voice_commands(state: tauri::State<AppState>) -> voicecommand::VoiceCommandStore {
    state.voice_commands.lock().unwrap().clone()
}

#[tauri::command]
fn save_voice_commands(state: tauri::State<AppState>, store: voicecommand::VoiceCommandStore) -> Result<(), String> {
    let path = state.voice_commands_path.lock().unwrap().clone();
    store.save(std::path::Path::new(&path))?;
    *state.voice_commands.lock().unwrap() = store;
    log::info!("Voice commands saved");
    Ok(())
}

// ---------------------------------------------------------------------------
// Snippets commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_snippets(state: tauri::State<AppState>) -> Vec<snippets::Snippet> {
    state.snippet_store.lock().unwrap().snippets.clone()
}

#[tauri::command]
fn save_snippets(state: tauri::State<AppState>, items: Vec<snippets::Snippet>) -> Result<(), String> {
    let mut store = state.snippet_store.lock().unwrap();
    store.snippets = items;
    let path = state.snippets_path.lock().unwrap().clone();
    store.save(std::path::Path::new(&path))?;
    log::info!("Snippets saved: {} items", store.snippets.len());
    Ok(())
}

#[tauri::command]
fn test_snippet_expansion(state: tauri::State<AppState>, text: String) -> String {
    let store = state.snippet_store.lock().unwrap();
    store.expand(&text)
}

#[tauri::command]
fn set_dictionary(state: tauri::State<AppState>, entries: Vec<dictionary::DictEntry>) -> Result<(), String> {
    let mut dict = state.dict.lock().unwrap();
    dict.entries = entries;
    let path = state.dict_path.lock().unwrap().clone();
    dict.save(std::path::Path::new(&path))?;
    log::info!("Dictionary saved: {} entries", dict.entries.len());
    Ok(())
}

#[tauri::command]
fn get_style(state: tauri::State<AppState>) -> String {
    let s = state.style.lock().unwrap();
    match *s {
        style::Style::Formal => "formal".to_string(),
        style::Style::Casual => "casual".to_string(),
        style::Style::Relaxed => "relaxed".to_string(),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            audio: Mutex::new(None),
            engine: Mutex::new(None),
            models_dir: Mutex::new(String::new()),
            vad_model_path: Mutex::new(String::new()),
            model_name: Mutex::new("Loading...".to_string()),
            style: Mutex::new(style::Style::default()),
            settings: Mutex::new(settings::Settings::default()),
            settings_path: Mutex::new(String::new()),
            db: Mutex::new(None),
            dict: Mutex::new(dictionary::Dictionary::default()),
            dict_path: Mutex::new(String::new()),
            is_first_run: Mutex::new(false),
            polish_enabled: Mutex::new(false),  // overwritten in setup from settings
            polish_prompt: Mutex::new(llm::DEFAULT_POLISH_PROMPT.to_string()),  // overwritten in setup
            usage: Mutex::new(usage::UsageData::new_week()),
            usage_path: Mutex::new(String::new()),
            snippet_store: Mutex::new(snippets::SnippetStore::default()),
            snippets_path: Mutex::new(String::new()),
            app_styles: Mutex::new(appdetect::AppStyleRules::default()),
            app_styles_path: Mutex::new(String::new()),
            voice_commands: Mutex::new(voicecommand::VoiceCommandStore::default()),
            voice_commands_path: Mutex::new(String::new()),
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // List available input devices
            let devices = audio::list_input_devices();
            log::info!("Found {} input devices", devices.len());

            // Start audio capture
            match audio::start_audio_capture(app.handle().clone()) {
                Ok(state) => {
                    log::info!("Audio capture initialized");
                    let app_state = app.state::<AppState>();
                    *app_state.audio.lock().unwrap() = Some(state);
                }
                Err(e) => {
                    log::error!("Failed to start audio: {}", e);
                }
            }

            // Set up models directory
            let app_data_dir = app.path().app_data_dir()
                .expect("Failed to get app data dir");
            let models_dir = app_data_dir.join("models");
            std::fs::create_dir_all(&models_dir).ok();

            let vad_model_path = models_dir.join("silero_vad.onnx");

            {
                let app_state = app.state::<AppState>();
                *app_state.models_dir.lock().unwrap() = models_dir.to_string_lossy().to_string();
                *app_state.vad_model_path.lock().unwrap() = vad_model_path.to_string_lossy().to_string();
            }

            // Load settings (detect first run)
            let settings_path = settings::Settings::path(&app_data_dir);
            let is_first_run = !settings_path.exists();
            let loaded_settings = settings::Settings::load(&settings_path);
            log::info!("Settings: style={}, model={}, first_run={}", loaded_settings.style, loaded_settings.model, is_first_run);

            // Save defaults on first run so next launch isn't "first run" again
            if is_first_run {
                let _ = loaded_settings.save(&settings_path);
                let app_state = app.state::<AppState>();
                *app_state.is_first_run.lock().unwrap() = true;
            }

            {
                let app_state = app.state::<AppState>();
                *app_state.settings_path.lock().unwrap() = settings_path.to_string_lossy().to_string();

                // Apply loaded style
                if let Ok(s) = serde_json::from_str::<style::Style>(&format!("\"{}\"", loaded_settings.style)) {
                    *app_state.style.lock().unwrap() = s;
                }

                // Apply loaded polish settings
                *app_state.polish_enabled.lock().unwrap() = loaded_settings.polish_enabled;
                *app_state.polish_prompt.lock().unwrap() = loaded_settings.polish_prompt.clone();

                *app_state.settings.lock().unwrap() = loaded_settings.clone();
            }

            // Load dictionary
            {
                let dict_path = app_data_dir.join("dictionary.json");
                let dict = dictionary::Dictionary::load(&dict_path);
                log::info!("Dictionary: {} entries", dict.entries.len());
                let app_state = app.state::<AppState>();
                *app_state.dict.lock().unwrap() = dict;
                *app_state.dict_path.lock().unwrap() = dict_path.to_string_lossy().to_string();
            }

            // Load snippets
            {
                let snippets_path = app_data_dir.join("snippets.json");
                let store = snippets::SnippetStore::load(&snippets_path);
                log::info!("Snippets: {} loaded", store.snippets.len());
                let app_state = app.state::<AppState>();
                *app_state.snippet_store.lock().unwrap() = store;
                *app_state.snippets_path.lock().unwrap() = snippets_path.to_string_lossy().to_string();
            }

            // Load per-app style rules
            {
                let app_styles_path = app_data_dir.join("app-styles.json");
                let rules = appdetect::AppStyleRules::load(&app_styles_path);
                log::info!("App style rules: {} rules, enabled={}", rules.rules.len(), rules.enabled);
                let app_state = app.state::<AppState>();
                *app_state.app_styles.lock().unwrap() = rules;
                *app_state.app_styles_path.lock().unwrap() = app_styles_path.to_string_lossy().to_string();
            }

            // Load voice commands
            {
                let vc_path = app_data_dir.join("voice-commands.json");
                let vc_store = voicecommand::VoiceCommandStore::load(&vc_path);
                log::info!("Voice commands: {} commands, enabled={}", vc_store.commands.len(), vc_store.enabled);
                let app_state = app.state::<AppState>();
                *app_state.voice_commands.lock().unwrap() = vc_store;
                *app_state.voice_commands_path.lock().unwrap() = vc_path.to_string_lossy().to_string();
            }

            // Load usage data
            {
                let usage_path = app_data_dir.join("usage.json");
                let mut usage = usage::UsageData::load(&usage_path);
                usage.ensure_current();
                log::info!("Usage: {}/{} words this week", usage.words_used, usage::FREE_TIER_WORDS);
                let app_state = app.state::<AppState>();
                *app_state.usage.lock().unwrap() = usage;
                *app_state.usage_path.lock().unwrap() = usage_path.to_string_lossy().to_string();
            }

            // Open transcript database
            {
                let db_path = app_data_dir.join("transcripts.db");
                match history::TranscriptDb::open(&db_path) {
                    Ok(db) => {
                        let app_state = app.state::<AppState>();
                        *app_state.db.lock().unwrap() = Some(db);
                    }
                    Err(e) => log::error!("Failed to open transcript DB: {}", e),
                }
            }

            log::info!("Models directory: {}", models_dir.display());
            log::info!("VAD model: {} (exists: {})", vad_model_path.display(), vad_model_path.exists());

            // Try to load models: Parakeet V3 (best) > Moonshine Tiny (fallback)
            {
                let app_state = app.state::<AppState>();
                let parakeet_dir = models_dir.join("parakeet-v3");
                let has_parakeet = parakeet_dir.join("encoder.int8.onnx").exists()
                    || parakeet_dir.join("encoder.onnx").exists();

                let tiny_dir = models_dir.join("moonshine-tiny");
                let has_tiny = tiny_dir.join("encoder.onnx").exists()
                    || tiny_dir.join("encoder.int8.onnx").exists()
                    || tiny_dir.join("encode.int8.onnx").exists()
                    || tiny_dir.join("encode.onnx").exists();

                let engine = if has_parakeet {
                    log::info!("Parakeet V3 found, loading...");
                    match SpeechEngine::parakeet(&models_dir) {
                        Ok(e) => Some(e),
                        Err(e) => {
                            log::warn!("Parakeet V3 load failed: {}, trying Moonshine Tiny", e);
                            let _ = app.emit("model-error", format!("Parakeet failed: {}. Falling back to Moonshine Tiny.", e));
                            if has_tiny {
                                SpeechEngine::moonshine(&models_dir, "tiny").ok()
                            } else { None }
                        }
                    }
                } else if has_tiny {
                    log::info!("Parakeet V3 not found, using Moonshine Tiny");
                    match SpeechEngine::moonshine(&models_dir, "tiny") {
                        Ok(e) => Some(e),
                        Err(e) => { log::warn!("Moonshine Tiny load failed: {}", e); None }
                    }
                } else {
                    log::info!("No models found yet. Download models to: {}", models_dir.display());
                    None
                };
                let model_name = match &engine {
                    Some(e) => match e.model_type() {
                        engine::ModelType::MoonshineTiny => "Moonshine Tiny",
                        engine::ModelType::MoonshineBase => "Moonshine Base",
                        engine::ModelType::MoonshineMedium => "Moonshine Medium",
                        engine::ModelType::Parakeet => "Parakeet V3",
                        engine::ModelType::ParakeetV2 => "Parakeet V2",
                        engine::ModelType::Whisper(name) => name.as_str(),
                        engine::ModelType::SenseVoice => "SenseVoice",
                    },
                    None => "No model loaded",
                };
                *app_state.model_name.lock().unwrap() = model_name.to_string();
                let _ = app.emit("model-loaded", model_name);
                *app_state.engine.lock().unwrap() = engine;
            }

            // Register global hotkey: Ctrl+Space
            let handle = app.handle().clone();

            app.handle().plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(move |_app, shortcut, event| {
                        let pressed = event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed;
                        let released = event.state == tauri_plugin_global_shortcut::ShortcutState::Released;

                        let app_state = handle.state::<AppState>();
                        let mode = app_state.settings.lock().unwrap().recording_mode.clone();
                        let is_toggle = mode == "toggle";

                        // Determine if we should start or stop
                        let should_start;
                        let should_stop;

                        if is_toggle {
                            // Toggle: press starts, press again stops. Ignore release.
                            if pressed {
                                let is_recording = app_state.audio.lock().unwrap()
                                    .as_ref()
                                    .map(|a| a.is_recording.load(Ordering::Relaxed))
                                    .unwrap_or(false);
                                should_start = !is_recording;
                                should_stop = is_recording;
                            } else {
                                should_start = false;
                                should_stop = false;
                            }
                        } else {
                            // Push-to-talk: press = start, release = stop
                            should_start = pressed;
                            should_stop = released;
                        }

                        if should_start {
                            // Start recording: clear buffer, set flag
                            let guard = app_state.audio.lock().unwrap();
                            if let Some(audio) = guard.as_ref() {
                                audio.recording_buffer.lock().unwrap().clear();
                                audio.is_recording.store(true, Ordering::Relaxed);
                                log::info!("Recording started ({}, shortcut: {:?})", mode, shortcut);
                            }
                            drop(guard);
                            let _ = handle.emit("recording-state", true);
                            overlay::show(&handle);
                        }

                        if should_stop {
                            // Stop recording: take buffer, resample, save WAV
                            let guard = app_state.audio.lock().unwrap();
                            if let Some(audio) = guard.as_ref() {
                                audio.is_recording.store(false, Ordering::Relaxed);

                                let samples: Vec<f32> = {
                                    let mut buf = audio.recording_buffer.lock().unwrap();
                                    std::mem::take(&mut *buf)
                                };
                                let source_rate = audio.sample_rate;

                                log::info!(
                                    "Recording stopped: {} samples ({:.1}s at {}Hz)",
                                    samples.len(),
                                    samples.len() as f32 / source_rate as f32,
                                    source_rate
                                );

                                drop(guard);
                                let _ = handle.emit("recording-state", false);

                                // Skip very short recordings (< 0.3s, likely accidental tap)
                                let min_samples = (source_rate as f32 * 0.3) as usize;
                                if samples.len() < min_samples {
                                    log::info!("Recording too short ({} samples, {:.1}s), skipping",
                                        samples.len(), samples.len() as f32 / source_rate as f32);
                                }

                                // Process in background: resample → VAD → transcribe
                                if samples.len() >= min_samples {
                                    let handle_clone = handle.clone();
                                    std::thread::spawn(move || { match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        // Debug: check raw audio RMS before resampling
                                        let raw_rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
                                        let raw_peak = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
                                        let nonzero = samples.iter().filter(|s| s.abs() > 0.0001).count();
                                        log::info!(
                                            "Raw audio: RMS={:.6}, Peak={:.6}, non-zero={}/{} ({:.1}%)",
                                            raw_rms, raw_peak, nonzero, samples.len(),
                                            100.0 * nonzero as f32 / samples.len() as f32
                                        );

                                        // 1. Resample to 16kHz
                                        let resampled = match recording::resample_to_16k(&samples, source_rate) {
                                            Ok(r) => {
                                                log::info!(
                                                    "Resampled: {} -> {} samples (16kHz, {:.1}s)",
                                                    samples.len(), r.len(), r.len() as f32 / 16000.0
                                                );
                                                r
                                            }
                                            Err(e) => { log::error!("Resampling failed: {}", e); return; }
                                        };

                                        // Debug: check resampled audio
                                        let res_rms = (resampled.iter().map(|s| s * s).sum::<f32>() / resampled.len() as f32).sqrt();
                                        let res_peak = resampled.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
                                        log::info!("Resampled audio: RMS={:.6}, Peak={:.6}", res_rms, res_peak);

                                        // Save debug WAV
                                        let wav_path = std::env::temp_dir().join("inkwell_debug.wav");
                                        let _ = recording::save_wav(&resampled, &wav_path);

                                        // 2. VAD: remove silence
                                        let app_state = handle_clone.state::<AppState>();
                                        let vad_path = app_state.vad_model_path.lock().unwrap().clone();
                                        let vad_threshold = app_state.settings.lock().unwrap().vad_threshold;
                                        let speech = if !vad_path.is_empty() && std::path::Path::new(&vad_path).exists() {
                                            match vad::remove_silence(&resampled, &vad_path, vad_threshold) {
                                                Ok(s) if !s.is_empty() => s,
                                                Ok(_) => {
                                                    log::warn!("VAD returned empty, using raw audio");
                                                    resampled.clone()
                                                }
                                                Err(e) => {
                                                    log::warn!("VAD failed ({}), using raw audio", e);
                                                    resampled.clone()
                                                }
                                            }
                                        } else {
                                            log::info!("No VAD model, skipping silence removal");
                                            resampled.clone()
                                        };

                                        // 3. Transcribe
                                        let engine_guard = app_state.engine.lock().unwrap();
                                        if let Some(engine) = engine_guard.as_ref() {
                                            match engine.transcribe(&speech) {
                                                Ok(text) => {
                                                    log::info!("Raw: \"{}\"", text);

                                                    // Check for voice commands before processing as dictation
                                                    {
                                                        let vc_store = app_state.voice_commands.lock().unwrap();
                                                        if let Some(cmd) = vc_store.detect(&text) {
                                                            log::info!("Voice command detected: {:?}", cmd.action);
                                                            let cmd_id = cmd.id.clone();
                                                            let action = cmd.action.clone();
                                                            drop(vc_store);

                                                            // Emit command event to frontend for execution
                                                            let _ = handle_clone.emit("voice-command", serde_json::json!({
                                                                "id": cmd_id,
                                                                "action": action,
                                                            }));

                                                            // Execute safe backend actions directly
                                                            match &action {
                                                                voicecommand::CommandAction::ChangeStyle { style: s } => {
                                                                    if let Ok(new_style) = serde_json::from_str::<style::Style>(&format!("\"{}\"", s)) {
                                                                        *app_state.style.lock().unwrap() = new_style;
                                                                        log::info!("Voice: style changed to {}", s);
                                                                    }
                                                                }
                                                                voicecommand::CommandAction::TogglePolish => {
                                                                    let mut enabled = app_state.polish_enabled.lock().unwrap();
                                                                    *enabled = !*enabled;
                                                                    log::info!("Voice: polish toggled to {}", *enabled);
                                                                }
                                                                _ => {} // Other actions handled by frontend
                                                            }

                                                            // Hide overlay and skip paste (return from closure early)
                                                            std::thread::sleep(std::time::Duration::from_millis(500));
                                                            overlay::hide(&handle_clone);
                                                            return;
                                                        }
                                                    }

                                                    // Apply style formatting (per-app override if enabled)
                                                    let current_style = {
                                                        let app_rules = app_state.app_styles.lock().unwrap();
                                                        if let Some(override_style) = app_rules.get_override() {
                                                            log::info!("Per-app style override: {}", override_style);
                                                            serde_json::from_str::<style::Style>(&format!("\"{}\"", override_style))
                                                                .unwrap_or_else(|_| app_state.style.lock().unwrap().clone())
                                                        } else {
                                                            app_state.style.lock().unwrap().clone()
                                                        }
                                                    };
                                                    let styled = current_style.format(&text);
                                                    log::info!("Styled ({:?}): \"{}\"", current_style, styled);

                                                    // Apply dictionary corrections
                                                    let dict = app_state.dict.lock().unwrap();
                                                    let styled = dict.apply(&styled);
                                                    drop(dict);

                                                    // Apply snippet expansions
                                                    let snippet_store = app_state.snippet_store.lock().unwrap();
                                                    let styled = snippet_store.expand(&styled);
                                                    drop(snippet_store);

                                                    // AI Polish (if enabled, async via tokio)
                                                    let polish_enabled = *app_state.polish_enabled.lock().unwrap();
                                                    let final_text = if polish_enabled && !styled.is_empty() {
                                                        let install_id = app_state.settings.lock().unwrap().install_id.clone();
                                                        let prompt = app_state.polish_prompt.lock().unwrap().clone();

                                                        // Check if user has a BYOK key for any provider
                                                        let byok_provider = ["groq", "openai", "anthropic", "openrouter"].iter()
                                                            .find(|p| keyring::Entry::new("inkwell", p).ok()
                                                                .and_then(|e| e.get_password().ok())
                                                                .map(|k| !k.is_empty()).unwrap_or(false))
                                                            .map(|s| s.to_string());

                                                        let styled_clone = styled.clone();
                                                        log::info!("AI Polish: sending to {}", byok_provider.as_deref().unwrap_or("proxy"));

                                                        // Block on async call (we're already in a spawned thread)
                                                        let rt = tokio::runtime::Handle::try_current()
                                                            .or_else(|_| {
                                                                tokio::runtime::Runtime::new().map(|rt| rt.handle().clone())
                                                            });

                                                        match rt {
                                                            Ok(handle) => {
                                                                let result = std::thread::spawn(move || {
                                                                    handle.block_on(async {
                                                                        if let Some(provider) = byok_provider {
                                                                            let api_key = keyring::Entry::new("inkwell", &provider)
                                                                                .ok().and_then(|e| e.get_password().ok()).unwrap_or_default();
                                                                            let cfg = llm::ProviderConfig {
                                                                                provider, api_key, custom_url: None, model: None,
                                                                            };
                                                                            let llm = llm::build_provider(cfg);
                                                                            llm.complete(&prompt, &styled_clone).await.map(|r| r.text).ok()
                                                                        } else {
                                                                            llm::call_proxy(&install_id, &styled_clone, &prompt).await
                                                                                .ok().and_then(|r| r.text)
                                                                        }
                                                                    })
                                                                }).join().ok().flatten();

                                                                match result {
                                                                    Some(polished) => {
                                                                        log::info!("AI Polish result: \"{}\"", polished);
                                                                        polished
                                                                    }
                                                                    None => {
                                                                        log::warn!("AI Polish failed, using unpolished text");
                                                                        styled
                                                                    }
                                                                }
                                                            }
                                                            Err(_) => {
                                                                log::warn!("No tokio runtime for AI Polish, skipping");
                                                                styled
                                                            }
                                                        }
                                                    } else {
                                                        styled
                                                    };

                                                    // Track usage when AI Polish was used
                                                    if polish_enabled && !final_text.is_empty() {
                                                        let mut usage = app_state.usage.lock().unwrap();
                                                        usage.ensure_current();
                                                        usage.add_words(&final_text);
                                                        let usage_path = app_state.usage_path.lock().unwrap().clone();
                                                        let _ = usage.save(std::path::Path::new(&usage_path));
                                                        log::info!("Usage: {}/{} words this week", usage.words_used, usage::FREE_TIER_WORDS);
                                                    }

                                                    let _ = handle_clone.emit("transcription", &final_text);

                                                    // Save to transcript history (skip empty)
                                                    if !final_text.is_empty() {
                                                        let duration_ms = (speech.len() as f32 / 16.0) as i64;
                                                        let style_name = format!("{:?}", current_style).to_lowercase();
                                                        let model_name = app_state.model_name.lock().unwrap().clone();
                                                        let db_guard = app_state.db.lock().unwrap();
                                                        if let Some(db) = db_guard.as_ref() {
                                                            let _ = db.insert(&final_text, &text, &style_name, &model_name, duration_ms);
                                                        }
                                                    }

                                                    // Paste into focused app (delay to ensure Ctrl from hotkey is released)
                                                    if !final_text.is_empty() {
                                                        std::thread::sleep(std::time::Duration::from_millis(100));
                                                        match paste::paste_text(&final_text) {
                                                            Ok(_) => {}
                                                            Err(e) => {
                                                                log::error!("Paste failed: {}", e);
                                                                let _ = handle_clone.emit("paste-error",
                                                                    "Paste failed (secure field?). Text is on your clipboard, Ctrl+V to paste manually.".to_string());
                                                            }
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    log::error!("Transcription failed: {}", e);
                                                    let _ = handle_clone.emit("transcription-error", e);
                                                }
                                            }
                                        } else {
                                            log::warn!("No speech engine loaded, skipping transcription");
                                            let _ = handle_clone.emit("recording-processed", speech.len());
                                        }

                                        // Hide overlay after processing
                                        std::thread::sleep(std::time::Duration::from_millis(800));
                                        overlay::hide(&handle_clone);
                                    })) {
                                        Ok(_) => {}
                                        Err(e) => {
                                            log::error!("Transcription thread panicked: {:?}", e);
                                        }
                                    }});
                                }
                            } else {
                                drop(guard);
                                let _ = handle.emit("recording-state", false);
                            }
                        }
                    })
                    .build(),
            )?;

            use tauri_plugin_global_shortcut::GlobalShortcutExt;
            let hotkey_str = {
                let app_state = app.state::<AppState>();
                let settings = app_state.settings.lock().unwrap();
                settings.hotkey.clone()
            };
            let shortcut: tauri_plugin_global_shortcut::Shortcut = hotkey_str.parse()
                .unwrap_or_else(|_| "ctrl+space".parse().unwrap());
            app.global_shortcut().register(shortcut)?;
            log::info!("Global hotkey registered: {}", hotkey_str);

            // System tray
            let show_item = MenuItemBuilder::with_id("show", "Show Inkwell").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let tray_menu = MenuBuilder::new(app)
                .item(&show_item)
                .separator()
                .item(&quit_item)
                .build()?;

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().cloned().unwrap())
                .tooltip("Inkwell")
                .menu(&tray_menu)
                .on_menu_event(|app, event| {
                    match event.id().as_ref() {
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click { button: tauri::tray::MouseButton::Left, .. } = event {
                        if let Some(window) = tray.app_handle().get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            log::info!("System tray initialized");

            // Hide to tray on close instead of quitting
            if let Some(window) = app.get_webview_window("main") {
                let w = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = w.hide();
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![audio::get_input_devices, get_model_name, get_installed_models, switch_model, transcribe_file, download_model, remove_model, set_style, get_style, get_settings, update_settings, get_transcripts, search_transcripts, delete_transcript, get_dictionary, set_dictionary, get_vad_threshold, set_vad_threshold, set_hotkey, check_first_run, export_transcripts, download_parakeet, save_api_key, get_api_key_status, get_polish_settings, set_polish_settings, get_usage, run_ai_polish, get_snippets, save_snippets, test_snippet_expansion, get_app_styles, save_app_styles, get_voice_commands, save_voice_commands])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
