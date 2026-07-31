use crate::{llm, overlay, paste, recording, sounds, style, vad, voicecommand, voiceedit, AppState, Intent};
use std::sync::atomic::Ordering;
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

            let (mode, show_overlay, edit_hotkey) = {
                let settings = app_state.settings.lock().unwrap();
                (
                    settings.recording_mode.clone(),
                    settings.show_overlay,
                    settings.edit_hotkey.clone(),
                )
            };

            // Which hotkey fired decides what the recording is for. Both share
            // one audio buffer: a second buffer would let a dictation and an
            // edit interleave and produce a transcript made of half of each.
            let is_edit = !edit_hotkey.trim().is_empty()
                && edit_hotkey
                    .parse::<tauri_plugin_global_shortcut::Shortcut>()
                    .map(|s| &s == shortcut)
                    .unwrap_or(false);

            // Edits are always push to talk. Toggle would leave the user
            // holding a captured selection with no visible indication that the
            // app is waiting for an instruction.
            let mode = if is_edit { "ptt".to_string() } else { mode };
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

            if should_start && is_edit {
                // Capture the selection before recording, while the user's
                // focus is still where they left it. Failing here must stop the
                // flow: recording an instruction with nothing to apply it to
                // wastes the user's breath and then says so 5 seconds later.
                match paste::copy_selection() {
                    Ok((Some(sel), previous)) => {
                        paste::restore_clipboard(previous);
                        log::info!("Voice edit: captured {} chars", sel.chars().count());
                        *app_state.edit_selection.lock().unwrap() = Some(sel);
                    }
                    Ok((None, previous)) => {
                        paste::restore_clipboard(previous);
                        log::info!("Voice edit: nothing selected");
                        let _ = handle.emit(
                            "voice-edit-error",
                            "Select some text first, then hold the edit hotkey and say what to change.",
                        );
                        return;
                    }
                    Err(e) => {
                        log::warn!("Voice edit: selection capture failed: {}", e);
                        let _ = handle.emit("voice-edit-error", format!("Could not read the selection: {}", e));
                        return;
                    }
                }
            }

            if should_start {
                *app_state.recording_intent.lock().unwrap() =
                    if is_edit { Intent::Edit } else { Intent::Dictate };
                let guard = app_state.audio.lock().unwrap();
                if let Some(audio) = guard.as_ref() {
                    {
                        let mut buf = audio.recording_buffer.lock().unwrap();
                        buf.clear();
                        // People start speaking as they press, not after: seed
                        // the take with the last 300ms the always-on pre-roll
                        // already heard, so the first word keeps its first
                        // phoneme. (The transcript history had "Cl." for
                        // "Claude" on exactly this failure.) Clamped to what an
                        // earlier take already claimed, so a rapid re-press
                        // does not re-transcribe the previous take's tail.
                        let w = audio.preroll.mark();
                        let want = audio.sample_rate * 3 / 10;
                        let from = w.saturating_sub(want).max(
                            audio.preroll_claimed.load(Ordering::Relaxed),
                        );
                        let lead = audio.preroll.since(from, w.saturating_sub(from));
                        audio.lead_len.store(lead.len(), Ordering::Relaxed);
                        buf.extend_from_slice(&lead);
                        // Reserve the first minute up front so the realtime
                        // callback is not growing the Vec sample by sample.
                        // Takes beyond 60s still grow by amortized doubling on
                        // the audio thread; rare enough to accept, not fixed.
                        let reserve = audio.sample_rate * 60;
                        if buf.capacity() < reserve {
                            let needed = reserve - buf.len();
                            buf.reserve(needed);
                        }
                    }
                    // Clear pause on start as well as stop. A flag surviving
                    // into the next session would make that dictation capture
                    // nothing, with no visible cause.
                    audio.is_paused.store(false, Ordering::Relaxed);
                    audio.is_recording.store(true, Ordering::Relaxed);
                    log::info!(
                        "Recording started ({}, shortcut: {:?})",
                        mode,
                        shortcut
                    );
                }
                drop(guard);
                sounds::play_dictation_start();
                let _ = handle.emit("recording-state", true);
                if show_overlay {
                    overlay::show(&handle);
                }
            }

            if should_stop {
                let guard = app_state.audio.lock().unwrap();
                if let Some(audio) = guard.as_ref() {
                    audio.is_recording.store(false, Ordering::Relaxed);
                    audio.is_paused.store(false, Ordering::Relaxed);

                    let mut samples: Vec<f32> = {
                        let mut buf = audio.recording_buffer.lock().unwrap();
                        std::mem::take(&mut *buf)
                    };
                    let source_rate = audio.sample_rate;
                    // The last word is still decaying when the key comes up.
                    // Mark the pre-roll position now; the worker sleeps briefly
                    // and then collects what the mic heard in the 300ms after
                    // release. Reading the tail from the global pre-roll,
                    // rather than keeping the shared buffer open, means a
                    // rapid next dictation can start immediately without
                    // racing this one's tail.
                    let tail_mark = audio.preroll.mark();
                    let tail_len = source_rate * 3 / 10;
                    let preroll = audio.preroll.clone();
                    // Claim through the tail so the next take's lead starts
                    // after it, and measure how much of this take was pre-roll
                    // seed rather than live speech.
                    audio.preroll_claimed.store(tail_mark + tail_len, Ordering::Relaxed);
                    let lead_len = audio.lead_len.swap(0, Ordering::Relaxed);
                    let intent = *app_state.recording_intent.lock().unwrap();

                    log::info!(
                        "Recording stopped: {} samples ({:.1}s at {}Hz)",
                        samples.len(),
                        samples.len() as f32 / source_rate as f32,
                        source_rate
                    );

                    drop(guard);
                    sounds::play_dictation_stop();
                    let _ = handle.emit("recording-state", false);

                    // Gate on LIVE audio, excluding the seeded lead: the buffer
                    // now always starts ~300ms full, so gating on total length
                    // would let a stray hotkey tap transcribe pre-press room
                    // noise and paste it somewhere.
                    let min_samples = (source_rate as f32 * 0.3) as usize;
                    let live_samples = samples.len().saturating_sub(lead_len);
                    let live_ms = (live_samples as f32 * 1000.0 / source_rate as f32) as i64;
                    if live_samples < min_samples {
                        log::info!(
                            "Recording too short ({} live samples, {:.1}s), skipping",
                            live_samples,
                            live_samples as f32 / source_rate as f32
                        );
                    } else {
                        let handle_clone = handle.clone();
                        std::thread::spawn(move || {
                            // Wait out the release tail, then append it. 350ms
                            // of added latency buys the final word's ending;
                            // transcription itself takes longer than this.
                            std::thread::sleep(std::time::Duration::from_millis(350));
                            samples.extend(preroll.since(tail_mark, tail_len));
                            match std::panic::catch_unwind(
                                std::panic::AssertUnwindSafe(|| {
                                    match intent {
                                        Intent::Dictate => process_recording(
                                            &handle_clone,
                                            samples,
                                            source_rate,
                                            live_ms,
                                        ),
                                        Intent::Edit => process_edit(
                                            &handle_clone,
                                            samples,
                                            source_rate,
                                        ),
                                    }
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

/// A missing VAD model degrades every dictation, so never let it pass quietly:
/// log it each time, and emit `vad-unavailable` for the UI. The emit is gated on
/// the reason changing so a permanently missing model isn't a toast per dictation.
fn warn_vad_unavailable(handle: &tauri::AppHandle) {
    use std::sync::atomic::AtomicU8;

    let status = vad::status();
    let reason = vad::unavailable_reason();
    log::warn!("VAD unavailable ({:?}): {}", status, reason);

    static NOTIFIED: AtomicU8 = AtomicU8::new(u8::MAX);
    let code = status as u8;
    if NOTIFIED.swap(code, Ordering::Relaxed) != code {
        let _ = handle.emit("vad-unavailable", reason);
    }
}

/// The core recording processing pipeline: resample → VAD → transcribe → style → dict → snippet → polish → paste.
/// `live_ms` is the wall-clock length of the user's actual press, excluding the
/// pre-roll lead and release tail this pipeline adds. History records that
/// rather than the length of whatever reached the recognizer, because the
/// latter moved every time VAD or padding changed, silently rewriting what
/// stored stats meant. This one answers "how long did I dictate", which is the
/// question the history view is actually asking.
fn process_recording(
    handle: &tauri::AppHandle,
    samples: Vec<f32>,
    source_rate: usize,
    live_ms: i64,
) {
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

    // Raw CoreAudio capture from the built-in mic array arrives with no AGC,
    // because Apple's voice processing is what normally lifts it, so real speech lands
    // around -65 dBFS. VAD then classifies all of it as silence and the
    // recogniser has to work from a near-flat signal. Normalise to a usable
    // peak first, with a noise floor so a genuinely silent room is not amplified
    // into hiss.
    let resampled = recording::normalize_peak(resampled);

    let app_state = handle.state::<AppState>();
    let (vad_threshold, debug_save_audio, show_overlay) = {
        let settings = app_state.settings.lock().unwrap();
        (
            settings.vad_threshold,
            settings.debug_save_audio,
            settings.show_overlay,
        )
    };

    // Opt-in debug WAV. Off by default: voice audio must not be left on disk.
    //
    // Written to a findable folder under Documents with a unique name per take,
    // because the previous fixed path in the system temp dir was wrong twice
    // over: every dictation overwrote the last, so a corpus could never
    // accumulate, and macOS periodically sweeps that directory, so the one file
    // that did exist could vanish before anyone looked. Accuracy work needs a
    // set of recordings, not the most recent one.
    if debug_save_audio {
        let dir = dirs_documents().join("Inkwell Debug Audio");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("Debug audio dir {} could not be created: {}", dir.display(), e);
        } else {
            // Counter, not a clock: names must be unique and ordered, and this
            // cannot collide with itself the way a second-resolution timestamp
            // can when two takes land in the same second.
            use std::sync::atomic::{AtomicU32, Ordering as O};
            static SEQ: AtomicU32 = AtomicU32::new(0);
            let existing = std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
            let n = existing as u32 + SEQ.fetch_add(1, O::Relaxed) + 1;
            let wav_path = dir.join(format!("take-{:04}.wav", n));
            match recording::save_wav(&resampled, &wav_path) {
                Ok(_) => log::warn!("Debug audio written to {}", wav_path.display()),
                Err(e) => log::warn!("Debug audio save failed: {}", e),
            }
        }
    }

    // 2. VAD: remove silence
    let vad_path = app_state.vad_model_path.lock().unwrap().clone();
    let speech = if !vad_path.is_empty() && std::path::Path::new(&vad_path).exists() {
        match vad::trim_silence(&resampled, &vad_path, vad_threshold) {
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
        warn_vad_unavailable(handle);
        resampled.clone()
    };

    // 3. Transcribe. The engine lives on its own thread; this blocks the
    // pipeline worker until it answers, which is what we want here.
    {
        // The dictionary's correction targets ride along as decoder hotwords,
        // so "Inkwell" can win during decoding instead of being patched after.
        let hotwords = app_state.dict.with(|d| d.hotwords());
        match app_state.engine.transcribe(speech.clone(), hotwords) {
            Ok(text) => {
                log::info!("Raw: \"{}\"", text);

                // Check for voice commands before processing as dictation
                {
                    let detected = app_state.voice_commands.with(|vc| vc.detect(&text).cloned());
                    if let Some(cmd) = detected {
                        log::info!("Voice command detected: {:?}", cmd.action);
                        let cmd_id = cmd.id.clone();
                        let action = cmd.action.clone();

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
                                // Pin the first mode written in that style.
                                // Writing app_state.style here (what this did
                                // before) changed a field the pipeline stopped
                                // reading when modes took over styling, so the
                                // command silently did nothing.
                                let target = app_state.modes.with(|store| {
                                    store
                                        .first_with_style(&s)
                                        .or_else(|| store.find_by_name(&s))
                                        .map(|m| (m.id.clone(), m.name.clone()))
                                });
                                match target {
                                    Some((id, name)) => {
                                        *app_state.pinned_mode.lock().unwrap() =
                                            Some(id);
                                        log::info!("Voice: pinned mode {}", name);
                                        let _ = handle.emit("mode-pinned", name);
                                    }
                                    None => log::warn!(
                                        "Voice: no mode is written in style {}",
                                        s
                                    ),
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
                        if show_overlay {
                            overlay::hide(handle);
                        }
                        return;
                    }
                }

                // Resolve the mode for the app about to receive the text. One
                // lookup now decides style, cleanup and polish, where three
                // separate settings used to decide them independently and could
                // not vary together.
                let active_mode = {
                    let app_id = crate::appdetect::foreground_app_id();
                    let pinned = app_state.pinned_mode.lock().unwrap().clone();
                    app_state.modes.with(|store| {
                        store
                            .resolve_with_override(app_id.as_deref(), pinned.as_deref())
                            .clone()
                    })
                };
                log::info!("Mode: {}", active_mode.name);

                // Strip disfluencies before styling, not after: removing a
                // leading "Um" leaves the next word lowercase, and style is the
                // stage that fixes casing.
                let text = if active_mode.remove_fillers {
                    let cleaned = crate::cleanup::remove_disfluencies(&text);
                    if cleaned != text {
                        log::info!("Disfluencies removed: {:?} -> {:?}", text, cleaned);
                    }
                    cleaned
                } else {
                    text
                };

                let current_style = serde_json::from_str::<style::Style>(
                    &format!("\"{}\"", active_mode.style),
                )
                .unwrap_or_else(|_| app_state.style.lock().unwrap().clone());

                let styled = current_style.format(&text);
                log::info!("Styled ({:?}): \"{}\"", current_style, styled);

                // Apply dictionary corrections
                let styled = app_state.dict.with(|d| d.apply(&styled));

                // Apply snippet expansions
                let styled = app_state.snippet_store.with(|s| s.expand(&styled));

                // AI Polish (BYOK only: no key configured means no polish)
                let polish_enabled = *app_state.polish_enabled.lock().unwrap();
                // The key is fetched here, together with the decision to polish,
                // because the fetch can fail: reading the keychain item makes
                // macOS ask the user, and the user can say no. This used to be
                // unwrap_or_default() further down, which turned a denial into an
                // empty key and sent the transcript to the provider anyway, to be
                // rejected with a 401. Saying no to the keychain must mean the
                // text never leaves the machine, not that it leaves and bounces.
                let byok_provider = if polish_enabled && !styled.is_empty() {
                    crate::polish::preferred_provider().and_then(|provider| {
                        match llm::api_key_for(&provider) {
                            Some(key) => Some((provider, key)),
                            None => {
                                log::warn!(
                                    "AI Polish skipped: keychain access for {} \
                                     denied or key missing; keeping the local \
                                     transcript",
                                    provider
                                );
                                None
                            }
                        }
                    })
                } else {
                    None
                };

                let final_text = match byok_provider {
                    Some((provider, api_key)) => {
                        let prompt =
                            app_state.polish_prompt.lock().unwrap().clone();
                        let styled_clone = styled.clone();
                        log::info!("AI Polish: sending to {}", provider);

                        match tokio::runtime::Runtime::new() {
                            Ok(runtime) => {
                                let result = std::thread::spawn(move || {
                                    runtime.block_on(async move {
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
                    }
                    None => {
                        if polish_enabled && !styled.is_empty() {
                            log::info!(
                                "AI Polish enabled but no API key configured, skipping"
                            );
                        }
                        styled
                    }
                };

                let _ = handle.emit("transcription", &final_text);

                // Save to transcript history (skip empty)
                if !final_text.is_empty() {
                    let duration_ms = live_ms;
                    let style_name =
                        format!("{:?}", current_style).to_lowercase();
                    let model_name = app_state.engine.name();
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
    }

    // Hide overlay after processing
    std::thread::sleep(std::time::Duration::from_millis(800));
    if show_overlay {
        overlay::hide(handle);
    }
}

/// The user's Documents folder, or the temp dir if HOME is somehow unset.
/// Debug recordings go somewhere a person can actually find and delete them.
fn dirs_documents() -> std::path::PathBuf {
    std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join("Documents"))
        .unwrap_or_else(|_| std::env::temp_dir())
}

/// The voice-edit pipeline: resample -> trim -> transcribe the instruction ->
/// apply it to the captured selection -> paste over that selection.
///
/// Deliberately not a branch inside `process_recording`. The two share three
/// early steps and disagree about everything after: no styling, no dictionary,
/// no snippets, no cleanup, and the output replaces text rather than being
/// inserted. Folding them together would mean a chain of `if intent ==` down
/// the length of the function.
fn process_edit(handle: &tauri::AppHandle, samples: Vec<f32>, source_rate: usize) {
    let app_state = handle.state::<AppState>();

    let selection = match app_state.edit_selection.lock().unwrap().take() {
        Some(s) => s,
        None => {
            log::warn!("Voice edit: no selection captured, nothing to do");
            return;
        }
    };

    let resampled = match recording::resample_to_16k(&samples, source_rate) {
        Ok(r) => recording::normalize_peak(r),
        Err(e) => {
            log::error!("Voice edit: resampling failed: {}", e);
            let _ = handle.emit("voice-edit-error", format!("Audio processing failed: {}", e));
            return;
        }
    };

    let (vad_threshold, vad_path) = {
        let settings = app_state.settings.lock().unwrap();
        (
            settings.vad_threshold,
            app_state.vad_model_path.lock().unwrap().clone(),
        )
    };
    let speech = match vad::trim_silence(&resampled, &vad_path, vad_threshold) {
        Ok(s) if !s.is_empty() => s,
        _ => resampled,
    };

    // The dictionary biases here too: an instruction naming a product the model
    // mishears would be applied as the wrong instruction.
    let hotwords = app_state.dict.with(|d| d.hotwords());
    let instruction = match app_state.engine.transcribe(speech, hotwords) {
        Ok(t) if !t.trim().is_empty() => t,
        Ok(_) => {
            log::info!("Voice edit: no instruction heard");
            let _ = handle.emit("voice-edit-error", "No instruction was heard, so nothing was changed.");
            return;
        }
        Err(e) => {
            log::error!("Voice edit: transcription failed: {}", e);
            let _ = handle.emit("voice-edit-error", format!("Could not transcribe the instruction: {}", e));
            return;
        }
    };
    log::info!("Voice edit instruction: \"{}\"", instruction);

    let provider = match crate::polish::preferred_provider() {
        Some(p) => p,
        None => {
            let _ = handle.emit(
                "voice-edit-error",
                "Voice editing needs an API key. Add one in Settings, AI.",
            );
            return;
        }
    };
    let api_key = match llm::api_key_for(&provider) {
        Some(k) => k,
        None => {
            let _ = handle.emit(
                "voice-edit-error",
                "Keychain access was denied, so the selection was left alone.",
            );
            return;
        }
    };

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            log::error!("Voice edit: no runtime: {}", e);
            return;
        }
    };
    let sel = selection.clone();
    let instr = instruction.clone();
    let result = std::thread::spawn(move || {
        runtime.block_on(async move {
            voiceedit::apply_edit(&provider, &api_key, None, &sel, &instr).await
        })
    })
    .join();

    match result {
        Ok(Ok(rewritten)) => {
            log::info!(
                "Voice edit: {} chars -> {} chars",
                selection.chars().count(),
                rewritten.chars().count()
            );
            // The selection is still selected in the target app, so pasting
            // replaces it. Nothing re-selects it first: doing so would need
            // synthetic arrow keys and would break wherever the user clicked
            // in the meantime.
            if let Err(e) = paste::paste_text(&rewritten) {
                log::error!("Voice edit: paste failed: {}", e);
                let _ = handle.emit("voice-edit-error", format!("Could not paste the rewrite: {}", e));
                return;
            }
            let _ = handle.emit("voice-edit-done", rewritten);
        }
        Ok(Err(e)) => {
            log::warn!("Voice edit failed: {}", e);
            let _ = handle.emit("voice-edit-error", e);
        }
        Err(_) => {
            log::error!("Voice edit: worker thread panicked");
            let _ = handle.emit("voice-edit-error", "The edit failed unexpectedly.");
        }
    }
}
