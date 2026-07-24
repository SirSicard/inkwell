use crate::{
    appdetect, audio, dictionary, engine, history, pipeline, settings, snippets,
    style, tray, vad, voicecommand, AppState,
};
use tauri::{Emitter, Manager};

/// The main app setup closure. Initializes audio, loads settings/models/data, registers hotkeys, tray, etc.
pub fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    if cfg!(debug_assertions) {
        app.handle().plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .build(),
        )?;
    }

    // Set up models directory
    let app_data_dir = app.path().app_data_dir().expect("Failed to get app data dir");
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
    log::info!(
        "Settings: style={}, model={}, first_run={}",
        loaded_settings.style,
        loaded_settings.model,
        is_first_run
    );

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
        if let Ok(s) =
            serde_json::from_str::<style::Style>(&format!("\"{}\"", loaded_settings.style))
        {
            *app_state.style.lock().unwrap() = s;
        }

        // Apply loaded polish settings
        *app_state.polish_enabled.lock().unwrap() = loaded_settings.polish_enabled;
        *app_state.polish_prompt.lock().unwrap() = loaded_settings.polish_prompt.clone();

        // Apply sound settings
        crate::sounds::set_dictation_sounds(loaded_settings.sound_dictation);

        *app_state.settings.lock().unwrap() = loaded_settings.clone();
    }

    // Audio starts after settings load so the saved mic_device can be honored.
    let devices = audio::list_input_devices();
    log::info!("Found {} input devices", devices.len());

    match audio::start_audio_capture(app.handle().clone(), &loaded_settings.mic_device) {
        Ok(state) => {
            log::info!("Audio capture initialized");
            let app_state = app.state::<AppState>();
            *app_state.audio.lock().unwrap() = Some(state);
        }
        Err(e) => {
            log::error!("Failed to start audio: {}", e);
        }
    }

    // Autostart: register the plugin, then reconcile the OS state with the setting.
    app.handle().plugin(tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        None,
    ))?;
    apply_start_on_boot(app.handle(), loaded_settings.start_on_boot);

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
        log::info!(
            "App style rules: {} rules, enabled={}",
            rules.rules.len(),
            rules.enabled
        );
        let app_state = app.state::<AppState>();
        *app_state.app_styles.lock().unwrap() = rules;
        *app_state.app_styles_path.lock().unwrap() =
            app_styles_path.to_string_lossy().to_string();
    }

    // Load voice commands
    {
        let vc_path = app_data_dir.join("voice-commands.json");
        let vc_store = voicecommand::VoiceCommandStore::load(&vc_path);
        log::info!(
            "Voice commands: {} commands, enabled={}",
            vc_store.commands.len(),
            vc_store.enabled
        );
        let app_state = app.state::<AppState>();
        *app_state.voice_commands.lock().unwrap() = vc_store;
        *app_state.voice_commands_path.lock().unwrap() = vc_path.to_string_lossy().to_string();
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
    log::info!(
        "VAD model: {} (exists: {})",
        vad_model_path.display(),
        vad_model_path.exists()
    );

    // Nothing else fetches the VAD model, so a fresh install has to get it here.
    ensure_vad_model(app.handle(), vad_model_path.clone());

    // Load the model the user actually chose, then fall back down the list.
    {
        let app_state = app.state::<AppState>();

        // Saved choice first, then the standard fallbacks. Dedup so a saved
        // "parakeet" isn't retried a second time as a fallback.
        let mut candidates: Vec<String> = vec![loaded_settings.model.clone()];
        for fallback in ["parakeet"] {
            if !candidates.iter().any(|c| c == fallback) {
                candidates.push(fallback.to_string());
            }
        }

        let mut loaded_engine = None;
        for (i, id) in candidates.iter().enumerate() {
            if !model_files_present(&models_dir, id) {
                log::info!("Model '{}' not installed, skipping", id);
                continue;
            }
            log::info!("Loading model '{}'...", id);
            match load_engine(&models_dir, id) {
                Ok(e) => {
                    loaded_engine = Some(e);
                    break;
                }
                Err(e) => {
                    log::warn!("Model '{}' load failed: {}", id, e);
                    // Only surface the saved choice failing — fallbacks are noise.
                    if i == 0 {
                        let _ = app.emit(
                            "model-error",
                            format!("{} failed to load: {}. Trying fallbacks.", id, e),
                        );
                    }
                }
            }
        }

        if loaded_engine.is_none() {
            log::info!(
                "No usable models found. Download models to: {}",
                models_dir.display()
            );
        }

        let model_name = match &loaded_engine {
            Some(e) => match e.model_type() {
                engine::ModelType::MoonshineTiny => "Moonshine Tiny",
                engine::ModelType::MoonshineBase => "Moonshine Base",
                engine::ModelType::MoonshineMedium => "Moonshine Medium",
                engine::ModelType::Parakeet => "Parakeet V3",
                engine::ModelType::ParakeetV2 => "Parakeet V2",
                engine::ModelType::Whisper(name) => name.as_str(),
                engine::ModelType::SenseVoice => "SenseVoice",
                engine::ModelType::CanaryFlash => "Canary Flash",
            },
            None => "No model loaded",
        };
        *app_state.model_name.lock().unwrap() = model_name.to_string();
        let _ = app.emit("model-loaded", model_name);
        *app_state.engine.lock().unwrap() = loaded_engine;
    }

    // Register global hotkey
    let handle = app.handle().clone();
    app.handle().plugin(pipeline::build_shortcut_plugin(handle))?;

    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let hotkey_str = {
        let app_state = app.state::<AppState>();
        let settings = app_state.settings.lock().unwrap();
        settings.hotkey.clone()
    };
    let shortcut: tauri_plugin_global_shortcut::Shortcut = hotkey_str
        .parse()
        .unwrap_or_else(|_| "ctrl+space".parse().unwrap());
    app.global_shortcut().register(shortcut)?;
    log::info!("Global hotkey registered: {}", hotkey_str);

    // System tray
    tray::setup_tray(app)?;

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
}

/// Silero VAD ships as a sherpa-onnx release asset, not on HuggingFace, so it
/// cannot reuse `commands::download_model`'s HF table and has its own URL.
/// Keep in sync with `scripts/download-models.sh` and `scripts/download-models.ps1`.
const SILERO_VAD_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx";

/// Fetch silero_vad.onnx if it is missing (~2MB). Without this, a fresh install
/// has no VAD model, and `pipeline::process_recording` skips silence removal —
/// invisibly. Runs in the background so it never delays the window, and reports
/// every outcome on `vad-status` so a failed fetch is surfaced, not swallowed.
fn ensure_vad_model(app: &tauri::AppHandle, path: std::path::PathBuf) {
    if path.exists() {
        vad::set_status(vad::VadStatus::Ready);
        return;
    }

    vad::set_status(vad::VadStatus::Downloading);
    log::info!("VAD model missing, fetching {}", SILERO_VAD_URL);

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = app.emit(
            "vad-status",
            serde_json::json!({ "state": "downloading", "percent": 0 }),
        );

        match fetch_vad_model(&app, &path).await {
            Ok(bytes) => {
                vad::set_status(vad::VadStatus::Ready);
                log::info!(
                    "VAD model ready: {} ({} bytes)",
                    path.display(),
                    bytes
                );
                let _ = app.emit("vad-status", serde_json::json!({ "state": "ready" }));
            }
            Err(e) => {
                vad::set_status(vad::VadStatus::Failed);
                log::error!("VAD model download failed: {}", e);
                let _ = app.emit(
                    "vad-status",
                    serde_json::json!({ "state": "failed", "error": e }),
                );
            }
        }
    });
}

/// Download the VAD model to `<path>.part` and rename on success: an interrupted
/// fetch must not leave a truncated file that the existence check calls installed.
async fn fetch_vad_model(
    app: &tauri::AppHandle,
    path: &std::path::Path,
) -> Result<u64, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create {}: {}", parent.display(), e))?;
    }

    let resp = reqwest::Client::new()
        .get(SILERO_VAD_URL)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {} for {}", resp.status(), SILERO_VAD_URL));
    }

    let tmp = path.with_extension("onnx.part");
    let written = match stream_to_file(app, resp, &tmp).await {
        Ok(n) => n,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };

    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Cannot move {} into place: {}", tmp.display(), e)
    })?;

    Ok(written)
}

/// Stream a response body to `dest`, emitting `vad-status` progress as it goes.
async fn stream_to_file(
    app: &tauri::AppHandle,
    resp: reqwest::Response,
    dest: &std::path::Path,
) -> Result<u64, String> {
    use futures_util::StreamExt;
    use std::io::Write;

    let total = resp.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(dest)
        .map_err(|e| format!("Cannot create {}: {}", dest.display(), e))?;

    let mut written: u64 = 0;
    let mut last_pct: u32 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Stream error: {}", e))?;
        file.write_all(&chunk)
            .map_err(|e| format!("Write error: {}", e))?;
        written += chunk.len() as u64;

        // Only on a whole-percent change — a 2MB body is hundreds of chunks.
        if total > 0 {
            let pct = (written * 100 / total).min(99) as u32;
            if pct > last_pct {
                last_pct = pct;
                let _ = app.emit(
                    "vad-status",
                    serde_json::json!({ "state": "downloading", "percent": pct }),
                );
            }
        }
    }

    file.flush().map_err(|e| format!("Flush error: {}", e))?;
    Ok(written)
}

/// On-disk directory for a model id. Must stay in sync with `commands::download_model`.
fn model_dir_name(model_id: &str) -> Option<&'static str> {
    Some(match model_id {
        "parakeet" => "parakeet-v3",
        "parakeet-v2" => "parakeet-v2",
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
        _ => return None,
    })
}

/// Cheap existence check so a missing model falls back instead of erroring at load.
fn model_files_present(models_dir: &std::path::Path, model_id: &str) -> bool {
    let dir = match model_dir_name(model_id) {
        Some(d) => models_dir.join(d),
        None => return false,
    };

    let candidates: Vec<String> = if let Some(variant) = model_id.strip_prefix("whisper-") {
        let variant = match variant {
            "distil-small-en" => "distil-small.en",
            "distil-medium-en" => "distil-medium.en",
            v => v,
        };
        vec![
            format!("{}-encoder.int8.onnx", variant),
            format!("{}-encoder.onnx", variant),
        ]
    } else if model_id == "sense-voice" {
        vec!["model.int8.onnx".into(), "model.onnx".into()]
    } else {
        vec![
            "encoder.int8.onnx".into(),
            "encoder.onnx".into(),
            "encode.int8.onnx".into(),
            "encode.onnx".into(),
        ]
    };

    candidates.iter().any(|f| dir.join(f).exists())
}

/// Build an engine for a model id. Mirrors `commands::switch_model`.
fn load_engine(
    models_dir: &std::path::Path,
    model_id: &str,
) -> Result<engine::SpeechEngine, String> {
    match model_id {
        "parakeet" => engine::SpeechEngine::parakeet(models_dir),
        "parakeet-v2" => engine::SpeechEngine::parakeet_v2(models_dir),
        "moonshine-base" => engine::SpeechEngine::moonshine(models_dir, "base"),
        "whisper-tiny" => engine::SpeechEngine::whisper(models_dir, "tiny"),
        "whisper-base" => engine::SpeechEngine::whisper(models_dir, "base"),
        "whisper-small" => engine::SpeechEngine::whisper(models_dir, "small"),
        "whisper-medium" => engine::SpeechEngine::whisper(models_dir, "medium"),
        "whisper-large-v3" => engine::SpeechEngine::whisper(models_dir, "large-v3"),
        "whisper-turbo" => engine::SpeechEngine::whisper(models_dir, "turbo"),
        "whisper-distil-small-en" => engine::SpeechEngine::whisper(models_dir, "distil-small.en"),
        "whisper-distil-medium-en" => engine::SpeechEngine::whisper(models_dir, "distil-medium.en"),
        "sense-voice" => engine::SpeechEngine::sense_voice(models_dir),
        other => Err(format!("Unknown model: {}", other)),
    }
}

/// Enable/disable launch-at-login. Best-effort: a failure here must not block the app.
pub fn apply_start_on_boot(app: &tauri::AppHandle, enabled: bool) {
    use tauri_plugin_autostart::ManagerExt;

    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    match result {
        Ok(_) => log::info!("Start on boot: {}", enabled),
        Err(e) => log::warn!("Start on boot ({}) failed: {}", enabled, e),
    }
}
