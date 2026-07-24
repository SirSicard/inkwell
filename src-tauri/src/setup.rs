use crate::{
    appdetect, audio, dictionary, engine, history, pipeline, settings, snippets,
    style, tray, usage, voicecommand, AppState,
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
        crate::sounds::set_agent_sounds(loaded_settings.sound_agent);

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

    // Load usage data
    {
        let usage_path = app_data_dir.join("usage.json");
        let mut usage = usage::UsageData::load(&usage_path);
        usage.ensure_current();
        log::info!(
            "Usage: {}/{} words this week",
            usage.words_used,
            usage::FREE_TIER_WORDS
        );
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
    log::info!(
        "VAD model: {} (exists: {})",
        vad_model_path.display(),
        vad_model_path.exists()
    );

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

        let loaded_engine = if has_parakeet {
            log::info!("Parakeet V3 found, loading...");
            match engine::SpeechEngine::parakeet(&models_dir) {
                Ok(e) => Some(e),
                Err(e) => {
                    log::warn!("Parakeet V3 load failed: {}, trying Moonshine Tiny", e);
                    let _ = app.emit(
                        "model-error",
                        format!("Parakeet failed: {}. Falling back to Moonshine Tiny.", e),
                    );
                    if has_tiny {
                        engine::SpeechEngine::moonshine(&models_dir, "tiny").ok()
                    } else {
                        None
                    }
                }
            }
        } else if has_tiny {
            log::info!("Parakeet V3 not found, using Moonshine Tiny");
            match engine::SpeechEngine::moonshine(&models_dir, "tiny") {
                Ok(e) => Some(e),
                Err(e) => {
                    log::warn!("Moonshine Tiny load failed: {}", e);
                    None
                }
            }
        } else {
            log::info!(
                "No models found yet. Download models to: {}",
                models_dir.display()
            );
            None
        };

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

    // Register agent hotkey (if enabled)
    {
        let app_state = app.state::<AppState>();
        let settings = app_state.settings.lock().unwrap();
        if settings.agent_enabled {
            let agent_hotkey_str = settings.agent_hotkey.clone();
            drop(settings);
            match agent_hotkey_str.parse::<tauri_plugin_global_shortcut::Shortcut>() {
                Ok(agent_shortcut) => {
                    app.global_shortcut().register(agent_shortcut)?;
                    log::info!("Agent hotkey registered: {}", agent_hotkey_str);
                }
                Err(e) => {
                    log::warn!("Failed to parse agent hotkey '{}': {}", agent_hotkey_str, e);
                }
            }
        }
    }

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
