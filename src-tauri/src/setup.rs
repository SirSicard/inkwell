use crate::{
    models,
    appdetect, audio, dictionary, history, modes, pipeline, settings, snippets,
    style, tray, vad, voicecommand, AppState,
};
use tauri::{Emitter, Manager};

/// The main app setup closure. Initializes audio, loads settings/models/data, registers hotkeys, tray, etc.
pub fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // Registered unconditionally. It used to be gated on debug_assertions, so
    // every build a user actually runs wrote nothing at all, and "it stopped
    // pasting" arrived with no trace to read. The transcript itself is not
    // written in release (see crate::redact); this records the shape of a run,
    // not its contents.
    //
    // The plugin's default max_file_size is 40 KB, which one afternoon of
    // dictation reaches, after which the log is worth little. Raised, and the
    // rotation made explicit so it keeps recent history without growing
    // without bound.
    app.handle().plugin(
        tauri_plugin_log::Builder::default()
            .level(log::LevelFilter::Info)
            .max_file_size(2_000_000)
            .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(3))
            .build(),
    )?;

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

    // Stuck-recording and idle-mic watchdog; see pipeline::watchdog_loop.
    {
        let wd_handle = app.handle().clone();
        std::thread::spawn(move || crate::pipeline::watchdog_loop(wd_handle));
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
        app.state::<AppState>().dict.set(dict, dict_path);
    }

    // Load snippets
    {
        let snippets_path = app_data_dir.join("snippets.json");
        let store = snippets::SnippetStore::load(&snippets_path);
        log::info!("Snippets: {} loaded", store.snippets.len());
        app.state::<AppState>().snippet_store.set(store, snippets_path);
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
        app.state::<AppState>().app_styles.set(rules, app_styles_path);
    }

    // Load modes, migrating an install that predates them.
    //
    // Migration runs only when no modes file exists, so it happens once and a
    // user who later empties their modes does not get the old rules resurrected.
    {
        let modes_path = app_data_dir.join("modes.json");
        let store = if modes_path.exists() {
            modes::ModeStore::load(&modes_path)
        } else {
            let app_state = app.state::<AppState>();
            let rules: Vec<(String, String)> = app_state
                .app_styles
                .with(|r| r.rules.iter().map(|x| (x.process_name.clone(), x.style.clone())).collect());
            let migrated = modes::ModeStore::migrate_from(
                &loaded_settings.style,
                loaded_settings.polish_enabled,
                &loaded_settings.polish_prompt,
                loaded_settings.remove_fillers,
                &rules,
            );
            log::info!(
                "Modes: migrated {} mode(s) from existing style and per-app rules",
                migrated.modes.len()
            );
            let _ = migrated.save(&modes_path);
            migrated
        };
        log::info!("Modes: {} loaded, default '{}'", store.modes.len(), store.default_id);
        app.state::<AppState>().modes.set(store, modes_path);
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
        app.state::<AppState>().voice_commands.set(vc_store, vc_path);
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

        // Saved choice first, then the default. Dedup so a saved "parakeet"
        // isn't retried a second time as a fallback.
        let mut candidates: Vec<&str> = vec![loaded_settings.model.as_str()];
        if !candidates.contains(&models::DEFAULT_MODEL_ID) {
            candidates.push(models::DEFAULT_MODEL_ID);
        }

        let mut loaded_name: Option<String> = None;
        for (i, id) in candidates.iter().enumerate() {
            let Some(spec) = models::find(id) else {
                // The catalogue was curated down from thirteen models, so a
                // saved choice can name one that no longer exists. Silently
                // loading something else would leave the user believing they
                // are on a model they are not, which is the kind of thing that
                // makes people distrust the transcript rather than the app.
                log::warn!("Saved model '{}' is no longer in the catalogue", id);
                if i == 0 {
                    let _ = app.emit(
                        "model-error",
                        format!(
                            "{} has been retired from the model list, so Inkwell loaded its \
                             default instead. Pick a model in Settings, Models. Its files are \
                             still on disk and can be deleted by hand from the models folder.",
                            id
                        ),
                    );
                }
                continue;
            };
            if !spec.is_installed(&models_dir) {
                log::info!("Model '{}' not installed, skipping", id);
                continue;
            }
            log::info!("Loading model '{}'...", id);
            match app_state.engine.load(spec, models_dir.clone()) {
                Ok(name) => {
                    loaded_name = Some(name);
                    break;
                }
                Err(e) => {
                    log::warn!("Model '{}' load failed: {}", id, e);
                    // Only surface the saved choice failing; fallbacks are noise.
                    if i == 0 {
                        let _ = app.emit(
                            "model-error",
                            format!("{} failed to load: {}. Trying fallbacks.", id, e),
                        );
                    }
                }
            }
        }

        if loaded_name.is_none() {
            log::info!(
                "No usable models found. Download models to: {}",
                models_dir.display()
            );
        }

        let _ = app.emit("model-loaded", app_state.engine.name());
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
    // A modifier-only token routes to the flagsChanged tap; anything else to
    // the OS hotkey API, falling back to a default if the string is garbage.
    // The tap path is non-fatal at startup: if the permission is gone, the
    // app must still come up so the user can fix it in Settings.
    let mut plugin_shortcut: Option<tauri_plugin_global_shortcut::Shortcut> = None;
    match crate::modkey::ModKey::from_token(&hotkey_str) {
        Some(mk) => match crate::modkey::set_binding(app.handle(), crate::modkey::SLOT_MAIN, Some(mk)) {
            Ok(_) => log::info!("Global hotkey armed via event tap: {}", hotkey_str),
            Err(e) => log::error!("Modifier hotkey '{}' failed: {}", hotkey_str, e),
        },
        None => {
            let shortcut: tauri_plugin_global_shortcut::Shortcut = hotkey_str
                .parse()
                .unwrap_or_else(|_| "ctrl+space".parse().unwrap());
            app.global_shortcut().register(shortcut)?;
            plugin_shortcut = Some(shortcut);
            log::info!("Global hotkey registered: {}", hotkey_str);
        }
    }

    // Voice editing's hotkey. Registered separately and non-fatally: a
    // collision with another app must not stop dictation from working, which
    // is what returning the error here would do.
    let edit_hotkey_str = {
        let app_state = app.state::<AppState>();
        let settings = app_state.settings.lock().unwrap();
        settings.edit_hotkey.clone()
    };
    if !edit_hotkey_str.trim().is_empty()
        && crate::modkey::ModKey::from_token(&edit_hotkey_str).is_some()
    {
        match crate::modkey::ModKey::from_token(&edit_hotkey_str)
            .filter(|_| edit_hotkey_str != hotkey_str)
            .map(|mk| crate::modkey::set_binding(app.handle(), crate::modkey::SLOT_EDIT, Some(mk)))
        {
            Some(Ok(_)) => log::info!("Edit hotkey armed via event tap: {}", edit_hotkey_str),
            Some(Err(e)) => log::warn!("Edit modifier hotkey '{}' failed: {}", edit_hotkey_str, e),
            None => log::warn!(
                "Edit hotkey '{}' is the same as the dictation hotkey; voice editing is off",
                edit_hotkey_str
            ),
        }
    } else if !edit_hotkey_str.trim().is_empty() {
        match edit_hotkey_str.parse::<tauri_plugin_global_shortcut::Shortcut>() {
            Ok(sc) if plugin_shortcut != Some(sc) => match app.global_shortcut().register(sc) {
                Ok(_) => log::info!("Edit hotkey registered: {}", edit_hotkey_str),
                Err(e) => log::warn!(
                    "Edit hotkey '{}' could not be registered ({}); voice editing is off",
                    edit_hotkey_str, e
                ),
            },
            Ok(_) => log::warn!(
                "Edit hotkey '{}' is the same as the dictation hotkey; voice editing is off",
                edit_hotkey_str
            ),
            Err(e) => log::warn!("Edit hotkey '{}' is not valid ({})", edit_hotkey_str, e),
        }
    }

    // System tray
    tray::setup_tray(app)?;

    // Hide to tray on close instead of quitting.
    //
    // Dropping to Accessory on the way out is what stops dictation stealing
    // focus: as a Regular app, showing the recording overlay activates Inkwell
    // and hiding it promotes the main window, so the synthetic Cmd+V landed in
    // Inkwell instead of whatever the user was typing into. An Accessory app
    // has no Dock tile and never takes focus on its own, which is the correct
    // shape for a tray-resident dictation tool anyway.
    if let Some(window) = app.get_webview_window("main") {
        let w = window.clone();
        let handle = app.handle().clone();
        window.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = w.hide();
                #[cfg(target_os = "macos")]
                let _ = handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
                #[cfg(not(target_os = "macos"))]
                let _ = &handle;
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
/// has no VAD model, and `pipeline::process_recording` skips silence removal,
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

        // Only on a whole-percent change; a 2MB body is hundreds of chunks.
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
