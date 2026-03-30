use crate::{llm, overlay, paste, recording, style, usage, vad, voicecommand, AppState};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

/// Build and register the global shortcut plugin with the hotkey handler (the core dictation pipeline).
pub fn build_shortcut_plugin(
    handle: tauri::AppHandle,
) -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(move |_app, shortcut, event| {
            let pressed =
                event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed;
            let released =
                event.state == tauri_plugin_global_shortcut::ShortcutState::Released;

            let app_state = handle.state::<AppState>();
            let mode = app_state.settings.lock().unwrap().recording_mode.clone();
            let is_toggle = mode == "toggle";

            let should_start;
            let should_stop;

            if is_toggle {
                if pressed {
                    let is_recording = app_state
                        .audio
                        .lock()
                        .unwrap()
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
                should_start = pressed;
                should_stop = released;
            }

            if should_start {
                let guard = app_state.audio.lock().unwrap();
                if let Some(audio) = guard.as_ref() {
                    audio.recording_buffer.lock().unwrap().clear();
                    audio.is_recording.store(true, Ordering::Relaxed);
                    log::info!(
                        "Recording started ({}, shortcut: {:?})",
                        mode,
                        shortcut
                    );

                    // Start pseudo-streaming partial transcription loop
                    let streaming_handle = handle.clone();
                    let buffer_ref = audio.recording_buffer.clone();
                    let sample_rate = audio.sample_rate;
                    let stop_streaming = Arc::new(AtomicBool::new(false));
                    *app_state.streaming_stop.lock().unwrap() =
                        Some(stop_streaming.clone());

                    std::thread::spawn(move || {
                        run_streaming_loop(
                            streaming_handle,
                            buffer_ref,
                            sample_rate,
                            stop_streaming,
                        );
                    });
                }
                drop(guard);
                let _ = handle.emit("recording-state", true);
                overlay::show(&handle);
            }

            if should_stop {
                // Stop the streaming loop first
                if let Some(stop_flag) =
                    app_state.streaming_stop.lock().unwrap().take()
                {
                    stop_flag.store(true, Ordering::Relaxed);
                }
                // Reset overlay to compact mode and clear text
                overlay::update_text(&handle, "");
                overlay::resize_for_text(&handle, false);

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

                    let min_samples = (source_rate as f32 * 0.3) as usize;
                    if samples.len() < min_samples {
                        log::info!(
                            "Recording too short ({} samples, {:.1}s), skipping",
                            samples.len(),
                            samples.len() as f32 / source_rate as f32
                        );
                    }

                    if samples.len() >= min_samples {
                        let handle_clone = handle.clone();
                        std::thread::spawn(move || {
                            match std::panic::catch_unwind(
                                std::panic::AssertUnwindSafe(|| {
                                    process_recording(
                                        &handle_clone,
                                        samples,
                                        source_rate,
                                    );
                                }),
                            ) {
                                Ok(_) => {}
                                Err(e) => {
                                    log::error!(
                                        "Transcription thread panicked: {:?}",
                                        e
                                    );
                                }
                            }
                        });
                    }
                } else {
                    drop(guard);
                    let _ = handle.emit("recording-state", false);
                }
            }
        })
        .build()
}

/// Pseudo-streaming: periodically snapshot the buffer and run offline transcription.
/// Emits "partial-transcription" events so the overlay can show live text.
fn run_streaming_loop(
    handle: tauri::AppHandle,
    buffer: Arc<Mutex<Vec<f32>>>,
    sample_rate: usize,
    stop: Arc<AtomicBool>,
) {
    let transcribing = Arc::new(AtomicBool::new(false));
    let min_samples_05s = (sample_rate as f32 * 0.5) as usize;

    // Initial delay before first partial (let some audio accumulate)
    std::thread::sleep(std::time::Duration::from_millis(800));

    while !stop.load(Ordering::Relaxed) {
        // Rate limit: skip if a previous partial transcription is still running
        if transcribing.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(100));
            continue;
        }

        // Clone current buffer snapshot
        let snapshot = {
            let buf = buffer.lock().unwrap();
            buf.clone()
        };

        // Skip if buffer too short (<0.5s)
        if snapshot.len() < min_samples_05s {
            std::thread::sleep(std::time::Duration::from_millis(200));
            continue;
        }

        // Run partial transcription in-place (same thread to keep it simple)
        transcribing.store(true, Ordering::Relaxed);

        let partial_text = run_partial_transcription(&handle, &snapshot, sample_rate);

        if let Some(text) = partial_text {
            if !text.is_empty() {
                let _ = handle.emit("partial-transcription", &text);
                overlay::update_text(&handle, &text);
                overlay::resize_for_text(&handle, true);
                log::debug!("Partial transcription: \"{}\"", text);
            }
        }

        transcribing.store(false, Ordering::Relaxed);

        // Wait before next cycle
        std::thread::sleep(std::time::Duration::from_millis(600));
    }

    log::info!("Streaming transcription loop stopped");
}

/// Run a single partial transcription on the given samples.
/// Returns None on error, Some(text) on success.
fn run_partial_transcription(
    handle: &tauri::AppHandle,
    samples: &[f32],
    source_rate: usize,
) -> Option<String> {
    // 1. Resample to 16kHz
    let resampled = match recording::resample_to_16k(samples, source_rate) {
        Ok(r) => r,
        Err(e) => {
            log::warn!("Partial resample failed: {}", e);
            return None;
        }
    };

    // 2. VAD
    let app_state = handle.state::<AppState>();
    let vad_path = app_state.vad_model_path.lock().unwrap().clone();
    let vad_threshold = app_state.settings.lock().unwrap().vad_threshold;
    let speech = if !vad_path.is_empty() && std::path::Path::new(&vad_path).exists() {
        match vad::remove_silence(&resampled, &vad_path, vad_threshold) {
            Ok(s) if !s.is_empty() => s,
            _ => resampled,
        }
    } else {
        resampled
    };

    if speech.is_empty() {
        return None;
    }

    // 3. Transcribe (brief lock on engine)
    let engine_guard = app_state.engine.lock().unwrap();
    let result = if let Some(engine) = engine_guard.as_ref() {
        engine.transcribe(&speech).ok()
    } else {
        None
    };
    drop(engine_guard);

    result
}

/// The core recording processing pipeline: resample → VAD → transcribe → style → dict → snippet → polish → paste.
fn process_recording(handle: &tauri::AppHandle, samples: Vec<f32>, source_rate: usize) {
    // Debug: check raw audio RMS before resampling
    let raw_rms =
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    let raw_peak = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    let nonzero = samples.iter().filter(|s| s.abs() > 0.0001).count();
    log::info!(
        "Raw audio: RMS={:.6}, Peak={:.6}, non-zero={}/{} ({:.1}%)",
        raw_rms,
        raw_peak,
        nonzero,
        samples.len(),
        100.0 * nonzero as f32 / samples.len() as f32
    );

    // 1. Resample to 16kHz
    let resampled = match recording::resample_to_16k(&samples, source_rate) {
        Ok(r) => {
            log::info!(
                "Resampled: {} -> {} samples (16kHz, {:.1}s)",
                samples.len(),
                r.len(),
                r.len() as f32 / 16000.0
            );
            r
        }
        Err(e) => {
            log::error!("Resampling failed: {}", e);
            return;
        }
    };

    // Debug: check resampled audio
    let res_rms =
        (resampled.iter().map(|s| s * s).sum::<f32>() / resampled.len() as f32).sqrt();
    let res_peak = resampled.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    log::info!(
        "Resampled audio: RMS={:.6}, Peak={:.6}",
        res_rms,
        res_peak
    );

    // Save debug WAV
    let wav_path = std::env::temp_dir().join("inkwell_debug.wav");
    let _ = recording::save_wav(&resampled, &wav_path);

    // 2. VAD: remove silence
    let app_state = handle.state::<AppState>();
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

                        let _ = handle.emit(
                            "voice-command",
                            serde_json::json!({
                                "id": cmd_id,
                                "action": action,
                            }),
                        );

                        match &action {
                            voicecommand::CommandAction::ChangeStyle {
                                style: s,
                            } => {
                                if let Ok(new_style) =
                                    serde_json::from_str::<style::Style>(
                                        &format!("\"{}\"", s),
                                    )
                                {
                                    *app_state.style.lock().unwrap() = new_style;
                                    log::info!("Voice: style changed to {}", s);
                                }
                            }
                            voicecommand::CommandAction::TogglePolish => {
                                let mut enabled =
                                    app_state.polish_enabled.lock().unwrap();
                                *enabled = !*enabled;
                                log::info!(
                                    "Voice: polish toggled to {}",
                                    *enabled
                                );
                            }
                            _ => {} // Other actions handled by frontend
                        }

                        std::thread::sleep(std::time::Duration::from_millis(500));
                        overlay::hide(handle);
                        return;
                    }
                }

                // Apply style formatting (per-app override if enabled)
                let current_style = {
                    let app_rules = app_state.app_styles.lock().unwrap();
                    if let Some(override_style) = app_rules.get_override() {
                        log::info!("Per-app style override: {}", override_style);
                        serde_json::from_str::<style::Style>(&format!(
                            "\"{}\"",
                            override_style
                        ))
                        .unwrap_or_else(|_| {
                            app_state.style.lock().unwrap().clone()
                        })
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
                    let install_id =
                        app_state.settings.lock().unwrap().install_id.clone();
                    let prompt =
                        app_state.polish_prompt.lock().unwrap().clone();

                    let byok_provider =
                        ["groq", "openai", "anthropic", "openrouter"]
                            .iter()
                            .find(|p| {
                                keyring::Entry::new("inkwell", p)
                                    .ok()
                                    .and_then(|e| e.get_password().ok())
                                    .map(|k| !k.is_empty())
                                    .unwrap_or(false)
                            })
                            .map(|s| s.to_string());

                    let styled_clone = styled.clone();
                    log::info!(
                        "AI Polish: sending to {}",
                        byok_provider.as_deref().unwrap_or("proxy")
                    );

                    let rt_result = tokio::runtime::Runtime::new();

                    match rt_result {
                        Ok(runtime) => {
                            let result = std::thread::spawn(move || {
                                runtime.block_on(async {
                                    if let Some(provider) = byok_provider {
                                        let api_key =
                                            keyring::Entry::new("inkwell", &provider)
                                                .ok()
                                                .and_then(|e| e.get_password().ok())
                                                .unwrap_or_default();
                                        let cfg = llm::ProviderConfig {
                                            provider,
                                            api_key,
                                            custom_url: None,
                                            model: None,
                                        };
                                        let llm = llm::build_provider(cfg);
                                        llm.complete(&prompt, &styled_clone)
                                            .await
                                            .map(|r| r.text)
                                            .ok()
                                    } else {
                                        match llm::call_proxy(
                                            &install_id,
                                            &styled_clone,
                                            &prompt,
                                        )
                                        .await
                                        {
                                            Ok(r) => {
                                                log::info!(
                                                    "Proxy response: {:?}",
                                                    r
                                                );
                                                r.text
                                            }
                                            Err(e) => {
                                                log::error!(
                                                    "Proxy call failed: {}",
                                                    e
                                                );
                                                None
                                            }
                                        }
                                    }
                                })
                            })
                            .join()
                            .ok()
                            .flatten();

                            match result {
                                Some(polished) => {
                                    log::info!(
                                        "AI Polish result: \"{}\"",
                                        polished
                                    );
                                    polished
                                }
                                None => {
                                    log::warn!(
                                        "AI Polish failed, using unpolished text"
                                    );
                                    styled
                                }
                            }
                        }
                        Err(_) => {
                            log::warn!(
                                "No tokio runtime for AI Polish, skipping"
                            );
                            styled
                        }
                    }
                } else {
                    styled
                };

                // Track usage when AI Polish was used
                if polish_enabled && !final_text.is_empty() {
                    let mut usage_data = app_state.usage.lock().unwrap();
                    usage_data.ensure_current();
                    usage_data.add_words(&final_text);
                    let usage_path =
                        app_state.usage_path.lock().unwrap().clone();
                    let _ =
                        usage_data.save(std::path::Path::new(&usage_path));
                    log::info!(
                        "Usage: {}/{} words this week",
                        usage_data.words_used,
                        usage::FREE_TIER_WORDS
                    );
                }

                let _ = handle.emit("transcription", &final_text);

                // Save to transcript history (skip empty)
                if !final_text.is_empty() {
                    let duration_ms =
                        (speech.len() as f32 / 16.0) as i64;
                    let style_name =
                        format!("{:?}", current_style).to_lowercase();
                    let model_name =
                        app_state.model_name.lock().unwrap().clone();
                    let db_guard = app_state.db.lock().unwrap();
                    if let Some(db) = db_guard.as_ref() {
                        let _ = db.insert(
                            &final_text,
                            &text,
                            &style_name,
                            &model_name,
                            duration_ms,
                        );
                    }
                }

                // Paste into focused app
                if !final_text.is_empty() {
                    std::thread::sleep(std::time::Duration::from_millis(
                        100,
                    ));
                    match paste::paste_text(&final_text) {
                        Ok(_) => {}
                        Err(e) => {
                            log::error!("Paste failed: {}", e);
                            let _ = handle.emit("paste-error",
                                "Paste failed (secure field?). Text is on your clipboard, Ctrl+V to paste manually.".to_string());
                        }
                    }
                }
            }
            Err(e) => {
                log::error!("Transcription failed: {}", e);
                let _ = handle.emit("transcription-error", e);
            }
        }
    } else {
        log::warn!("No speech engine loaded, skipping transcription");
        let _ = handle.emit("recording-processed", speech.len());
    }

    // Hide overlay after processing
    std::thread::sleep(std::time::Duration::from_millis(800));
    overlay::hide(handle);
}
