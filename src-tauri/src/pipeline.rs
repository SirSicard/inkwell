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

            // Which hotkey fired decides what the recording is for. Both share
            // one audio buffer: a second buffer would let a dictation and an
            // edit interleave and produce a transcript made of half of each.
            let app_state = handle.state::<AppState>();
            let edit_hotkey = app_state.settings.lock().unwrap().edit_hotkey.clone();
            let is_edit = !edit_hotkey.trim().is_empty()
                && edit_hotkey
                    .parse::<tauri_plugin_global_shortcut::Shortcut>()
                    .map(|s| &s == shortcut)
                    .unwrap_or(false);

            on_hotkey(&handle, is_edit, pressed);
        })
        .build()
}

/// One hotkey transition: `pressed` true on the way down, false on the way up.
///
/// Extracted from the plugin handler so a second event source can drive the
/// identical path: modifier-only hotkeys (Fn, right Command) never reach the
/// OS hotkey API and arrive from the flagsChanged event tap instead
/// (modkey.rs). Whatever fires the key, the recording semantics must be the
/// same code, or the two kinds of hotkey drift apart bug by bug.
/// What a hotkey transition means, given the mode and what is already running.
///
/// Pure, and separate from `on_hotkey`, because the rest of that function is
/// I/O: it wants an AppHandle, a live audio stream and a settings lock, none
/// of which a unit test can hand it. Every bug this logic has had was in the
/// decision rather than the plumbing, so the decision is what gets tested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    pub start: bool,
    pub stop: bool,
}

pub fn decide_transition(
    pressed: bool,
    is_recording: bool,
    mode: &str,
    is_edit: bool,
) -> Transition {
    // Edits are always push to talk. Toggle would leave the user holding a
    // captured selection with no visible sign the app is waiting for an
    // instruction, so the mode setting does not apply to them.
    let toggle = !is_edit && mode == "toggle";

    Transition {
        // Identical in both modes, and guarded on state rather than on the
        // event: a Pressed arriving while already recording used to clear the
        // buffer and start over, silently discarding whatever had been said.
        start: pressed && !is_recording,
        // Toggle stops on the next press; push-to-talk stops on release. Both
        // require something to actually be running, or a stray Released ran
        // the whole stop path against an empty buffer.
        stop: if toggle { pressed && is_recording } else { !pressed && is_recording },
    }
}

pub fn on_hotkey(handle: &tauri::AppHandle, is_edit: bool, pressed: bool) {
    let app_state = handle.state::<AppState>();

    let (mode, show_overlay) = {
        let settings = app_state.settings.lock().unwrap();
        (settings.recording_mode.clone(), settings.show_overlay)
    };

    let is_recording = app_state
        .audio
        .lock()
        .unwrap()
        .as_ref()
        .map(|a| a.is_recording.load(Ordering::Relaxed))
        .unwrap_or(false);

    let Transition { start: should_start, stop: should_stop } =
        decide_transition(pressed, is_recording, &mode, is_edit);

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
        *app_state.mic_last_used.lock().unwrap() = std::time::Instant::now();

        // The idle watchdog may have dropped the capture stream to
        // release the microphone; reopen it before recording. A
        // reopened stream starts with an empty pre-roll, so the first
        // dictation after a long idle loses its 300ms lead. That is
        // the price of the machine being able to sleep, paid once per
        // idle period rather than per dictation.
        let need_open = app_state.audio.lock().unwrap().is_none();
        if need_open {
            let mic_device =
                app_state.settings.lock().unwrap().mic_device.clone();
            match crate::audio::start_audio_capture(handle.clone(), &mic_device) {
                Ok(s) => {
                    *app_state.audio.lock().unwrap() = Some(s);
                    log::info!("Mic reopened after idle release");
                }
                Err(e) => {
                    log::error!("Mic reopen failed: {}", e);
                    let _ = handle.emit(
                        "mic-error",
                        format!("Could not open the microphone: {}", e),
                    );
                    return;
                }
            }
        }

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
            *app_state.recording_started.lock().unwrap() =
                Some(std::time::Instant::now());
            log::info!("Recording started ({}, edit={})", mode, is_edit);
        }
        drop(guard);
        sounds::play_dictation_start();
        let _ = handle.emit("recording-state", true);
        if show_overlay {
            overlay::show(&handle);
        }
        spawn_partial_feeder(&handle);
    }

    if should_stop {
        stop_and_process(&handle, "hotkey");
    }
}

/// How often the feeder hands new audio to the streaming recognizer. The
/// model's own chunk size dominates the latency the user sees, so polling
/// faster buys nothing and wakes a thread for it.
const PARTIAL_FEED_INTERVAL_MS: u64 = 100;

/// Pump the in-flight take into the streaming recognizer so the overlay can
/// show words while the key is still held.
///
/// A no-op unless the setting is on *and* the model is loaded, which is the
/// normal case: this costs nothing when nobody asked for it.
///
/// It reads `recording_buffer` rather than the pre-roll ring on purpose. That
/// buffer is exactly what the offline pass will transcribe, lead included, so
/// the partials describe the same audio rather than a near-miss of it.
fn spawn_partial_feeder(handle: &tauri::AppHandle) {
    {
        let app_state = handle.state::<AppState>();
        {
            let settings = app_state.settings.lock().unwrap();
            if !settings.show_partials {
                return;
            }
            // Partials are drawn on the overlay and nowhere else, so with the
            // overlay off there is nothing to draw on. The settings UI only
            // offers this toggle while the overlay is on, but the two are
            // separate stored values: turning the overlay off afterwards left
            // a second model decoding every dictation into a window that was
            // never shown.
            if !settings.show_overlay {
                return;
            }
        }
        if !app_state.streaming.is_ready() {
            return;
        }
    }

    let handle = handle.clone();
    let spawned = std::thread::Builder::new()
        .name("partial-feeder".into())
        .spawn(move || {
            let app_state = handle.state::<AppState>();

            let (buf, rate) = {
                let guard = app_state.audio.lock().unwrap();
                let Some(audio) = guard.as_ref() else { return };
                (audio.recording_buffer.clone(), audio.sample_rate)
            };

            // The take this feeder belongs to. `recording_started` is set once
            // per take and cleared on stop, so comparing against it is a
            // generation check: a feeder still between polls when the next take
            // begins sees a different value and exits instead of feeding the
            // previous take's audio into the new utterance.
            let take = *app_state.recording_started.lock().unwrap();
            if take.is_none() {
                return;
            }

            app_state.streaming.begin(rate);

            let mut consumed = 0usize;
            loop {
                if *app_state.recording_started.lock().unwrap() != take {
                    break;
                }
                let chunk = {
                    let b = buf.lock().unwrap();
                    if b.len() > consumed {
                        b[consumed..].to_vec()
                    } else {
                        Vec::new()
                    }
                };
                if !chunk.is_empty() {
                    consumed += chunk.len();
                    app_state.streaming.feed(chunk);
                }
                std::thread::sleep(std::time::Duration::from_millis(
                    PARTIAL_FEED_INTERVAL_MS,
                ));
            }

            app_state.streaming.end();
        });

    if let Err(e) = spawned {
        // Partials are decoration; losing them must never take the dictation
        // with it.
        log::warn!("Could not start the partial feeder: {}", e);
    }
}

/// Stop the in-flight recording and hand it to the pipeline.
///
/// Factored out of the hotkey handler so the watchdog can run the identical
/// path when the hotkey's Released event never arrives. The stuck case and the
/// normal case must not drift apart: whatever the user said is processed and
/// pasted either way, not discarded.
pub fn stop_and_process(handle: &tauri::AppHandle, reason: &str) {
    let app_state = handle.state::<AppState>();
    *app_state.mic_last_used.lock().unwrap() = std::time::Instant::now();
    *app_state.recording_started.lock().unwrap() = None;
    log::info!("Stopping recording ({})", reason);

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


/// Runs for the life of the app, checking every five seconds for the two
/// failure modes that only happen when nobody is looking.
///
/// 1. A push-to-talk recording that has run implausibly long, because the
///    hotkey's Released event never arrived. Seen in the field: the overlay
///    showed live audio levels for minutes while no key did anything, since
///    the handler was waiting for an event that was already lost. The
///    recording is stopped and processed, not discarded; the user did speak.
///    Toggle mode is exempt, because there a long take is plausibly
///    deliberate and the stop is its own press. Edits are always
///    push-to-talk, so they are covered regardless of the mode setting.
///
/// 2. An idle capture stream. Holding the mic open held a
///    PreventUserIdleSystemSleep assertion through coreaudiod (verified with
///    pmset -g assertions against Inkwell's pid), so the machine never
///    idle-slept, never auto-locked, and showed the mic indicator all day.
///    After the configured idle time the stream is dropped, which releases
///    the device and the assertion; the next hotkey press reopens it.
pub fn watchdog_loop(handle: tauri::AppHandle) {
    const STUCK_AFTER: std::time::Duration = std::time::Duration::from_secs(180);

    loop {
        std::thread::sleep(std::time::Duration::from_secs(5));
        let app_state = handle.state::<AppState>();

        let recording_for = app_state
            .recording_started
            .lock()
            .unwrap()
            .map(|t| t.elapsed());

        if let Some(elapsed) = recording_for {
            let intent = *app_state.recording_intent.lock().unwrap();
            let mode = app_state.settings.lock().unwrap().recording_mode.clone();
            let ptt = intent == Intent::Edit || mode != "toggle";
            if ptt && elapsed > STUCK_AFTER {
                log::warn!(
                    "Recording ran {:.0}s with no Released event; forcing a stop",
                    elapsed.as_secs_f32()
                );
                stop_and_process(&handle, "watchdog: release event never arrived");
            }
            // Recording (or just force-stopped): never idle-release beneath it.
            continue;
        }

        let idle_mins = app_state.settings.lock().unwrap().mic_idle_release_mins;
        if idle_mins == 0 {
            continue;
        }
        if app_state.mic_last_used.lock().unwrap().elapsed()
            < std::time::Duration::from_secs(idle_mins * 60)
        {
            continue;
        }
        let mut guard = app_state.audio.lock().unwrap();
        let releasable = guard
            .as_ref()
            .map(|a| !a.is_recording.load(Ordering::Relaxed))
            .unwrap_or(false);
        if releasable {
            *guard = None;
            log::info!(
                "Mic released after {}min idle; the next dictation reopens it",
                idle_mins
            );
        }
    }
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
            // Highest existing number plus one. The previous version added a
            // per-session counter to the directory count, so both grew with
            // every take and the names went 1, 3, 5, 7: unique and ordered, but
            // visibly wrong, and it made "take-0013" impossible to relate to
            // "the thirteenth thing I said".
            //
            // Reads the numbers rather than counting entries, so the paired
            // .txt files a corpus needs do not shift the numbering either.
            let next = std::fs::read_dir(&dir)
                .map(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .filter_map(|e| {
                            let name = e.file_name().to_string_lossy().into_owned();
                            name.strip_prefix("take-")
                                .and_then(|r| r.split('.').next())
                                .and_then(|d| d.parse::<u32>().ok())
                        })
                        .max()
                        .unwrap_or(0)
                })
                .unwrap_or(0)
                + 1;
            let wav_path = dir.join(format!("take-{:04}.wav", next));
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
                log::info!("Raw: {}", crate::redact(&text));

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
                        // Through redact(), like every other line that touches
                        // transcript text. Written with {:?} on the raw strings
                        // this was the one place a release build put a whole
                        // dictation into a log file that outlives the delete
                        // button in the Dashboard, in an app whose claim is
                        // that your words stay where you can see them.
                        log::info!(
                            "Disfluencies removed: {} -> {}",
                            crate::redact(&text),
                            crate::redact(&cleaned)
                        );
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
                log::info!("Styled ({:?}): {}", current_style, crate::redact(&styled));

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
                                        // The error is kept, not dropped. It
                                        // used to end in .ok(), so a key that
                                        // had expired or a model the provider
                                        // had retired failed on every single
                                        // dictation and the log said only
                                        // "AI Polish failed", with the reason
                                        // thrown away at the one point it was
                                        // known.
                                        llm.complete(&prompt, &styled_clone)
                                            .await
                                            .map(|r| r.text)
                                    })
                                })
                                .join()
                                .unwrap_or_else(|_| {
                                    Err("the polish thread panicked".to_string())
                                });

                                match result {
                                    Ok(polished) => {
                                        log::info!(
                                            "AI Polish result: {}",
                                            crate::redact(&polished)
                                        );
                                        polished
                                    }
                                    Err(e) => {
                                        // The provider's own words. This is the
                                        // difference between "polish is broken"
                                        // and knowing which key or model to fix.
                                        log::warn!(
                                            "AI Polish failed ({}), using unpolished text",
                                            e
                                        );
                                        let _ = handle.emit("polish-error", e);
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
                        // Not discarded. A failed insert used to be the one
                        // failure in this function that logged nothing at all,
                        // and the dictation still pasted, so the only symptom
                        // was history quietly missing entries. Worse, the tray's
                        // "Copy Last Transcript" then hands back the previous
                        // take with full confidence, which is wrong rather than
                        // merely absent.
                        if let Err(e) = db.insert(
                            &final_text,
                            &text,
                            &style_name,
                            &model_name,
                            duration_ms,
                        ) {
                            log::error!("History save failed: {}", e);
                            let _ = handle.emit(
                                "history-error",
                                format!("This dictation was pasted but not saved to history: {}", e),
                            );
                        }
                    }
                }

                // Paste into focused app
                if !final_text.is_empty() {
                    // One trailing space, so back-to-back dictations do not
                    // run together (dictate, dictate again, and the words
                    // fused). Trailing rather than leading: a leading space is
                    // wrong at the start of an empty field, which is where a
                    // first dictation lands. Added at the paste only; the
                    // history row and the transcription event stay exactly
                    // what was said.
                    let to_paste = if app_state.settings.lock().unwrap().append_space {
                        format!("{} ", final_text)
                    } else {
                        final_text.clone()
                    };
                    std::thread::sleep(std::time::Duration::from_millis(
                        100,
                    ));
                    match paste::paste_text(&to_paste) {
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
    log::info!("Voice edit instruction: {}", crate::redact(&instruction));

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

#[cfg(test)]
mod transition_tests {
    use super::{decide_transition, Transition};

    fn t(pressed: bool, recording: bool, mode: &str, edit: bool) -> (bool, bool) {
        let Transition { start, stop } = decide_transition(pressed, recording, mode, edit);
        (start, stop)
    }

    #[test]
    fn ptt_press_starts_release_stops() {
        assert_eq!(t(true, false, "ptt", false), (true, false));
        assert_eq!(t(false, true, "ptt", false), (false, true));
    }

    /// The bug that silently discarded a whole dictation: a second Pressed
    /// arriving mid-recording used to clear the buffer and begin again. It is
    /// also what happens when the Released event is lost, so this is the case
    /// the watchdog exists to clean up after.
    #[test]
    fn ptt_press_while_recording_does_nothing() {
        assert_eq!(t(true, true, "ptt", false), (false, false));
    }

    /// A Released with nothing in flight used to run the entire stop path
    /// against an empty buffer.
    #[test]
    fn ptt_release_while_idle_does_nothing() {
        assert_eq!(t(false, false, "ptt", false), (false, false));
    }

    #[test]
    fn toggle_stops_on_the_next_press_not_on_release() {
        assert_eq!(t(true, false, "toggle", false), (true, false));
        assert_eq!(t(true, true, "toggle", false), (false, true));
        // Releasing a toggle hotkey is not an event at all.
        assert_eq!(t(false, true, "toggle", false), (false, false));
        assert_eq!(t(false, false, "toggle", false), (false, false));
    }

    /// Voice edits capture a selection first; leaving the user in a toggle
    /// recording with no visible sign the app is waiting would strand them.
    #[test]
    fn edits_are_push_to_talk_even_when_the_setting_says_toggle() {
        assert_eq!(t(true, false, "toggle", true), (true, false));
        assert_eq!(t(false, true, "toggle", true), (false, true));
        assert_eq!(t(true, true, "toggle", true), (false, false));
    }

    /// An unknown mode string must behave like push to talk rather than
    /// falling into some third state, since the setting is free text on disk
    /// and a hand-edited settings.json should not brick the hotkey.
    #[test]
    fn unknown_mode_falls_back_to_push_to_talk() {
        assert_eq!(t(true, false, "", false), (true, false));
        assert_eq!(t(false, true, "wibble", false), (false, true));
    }

    /// Starting and stopping in one transition would run the stop path against
    /// a buffer the start path just cleared. Checked across the whole input
    /// space rather than on the cases someone thought to write down.
    #[test]
    fn never_starts_and_stops_at_once() {
        for pressed in [true, false] {
            for recording in [true, false] {
                for mode in ["ptt", "toggle", "", "nonsense"] {
                    for edit in [true, false] {
                        let (start, stop) = t(pressed, recording, mode, edit);
                        assert!(!(start && stop), "both for {pressed} {recording} {mode} {edit}");
                        // Nothing can start unless the mic is free, and nothing
                        // can stop unless something is running.
                        if start { assert!(!recording); }
                        if stop { assert!(recording); }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod log_privacy_tests {
    /// A release build must never write a dictation to the log file.
    ///
    /// A source lint rather than a behaviour test because that is where the bug
    /// lives: `redact()` existed, was tested, and was used on every line but
    /// one. `"Disfluencies removed: {:?} -> {:?}"` put two full copies of the
    /// user's sentence into a file that outlives the delete button, and it
    /// shipped.
    ///
    /// The first version of this lint scanned single physical lines and was
    /// close to useless: this codebase writes anything with two arguments
    /// across several lines, so `log::warn!(` alone on a line has no string
    /// literal on it and was skipped entirely. Verified by planting a
    /// multi-line leak of a watched binding and watching the lint pass. It now
    /// joins each `log::` invocation into one logical statement first, and
    /// covers every file that handles transcript text rather than this one.
    #[test]
    fn no_log_line_formats_transcript_text_directly() {
        const TEXT_BINDINGS: &[&str] = &[
            "text", "cleaned", "styled", "polished", "transcript", "raw",
            "instruction", "selection", "sel", "expanded", "final_text",
            "hypothesis", "partial", "content", "body",
        ];
        const SOURCES: &[(&str, &str)] = &[
            ("pipeline.rs", include_str!("pipeline.rs")),
            ("voiceedit.rs", include_str!("voiceedit.rs")),
            ("voicecommand.rs", include_str!("voicecommand.rs")),
            ("snippets.rs", include_str!("snippets.rs")),
            ("history.rs", include_str!("history.rs")),
            ("streaming.rs", include_str!("streaming.rs")),
            ("llm.rs", include_str!("llm.rs")),
            ("polish.rs", include_str!("polish.rs")),
        ];

        let mut offenders = Vec::new();

        for (name, src) in SOURCES {
            let lines: Vec<&str> = src.lines().collect();
            let mut i = 0;
            while i < lines.len() {
                let t = lines[i].trim();
                if !t.contains("log::") || t.starts_with("//") || t.starts_with("///") {
                    i += 1;
                    continue;
                }
                // Join the whole invocation: keep taking lines until the
                // parentheses balance. This is what the first version missed.
                let start_line = i;
                let mut stmt = String::new();
                let mut depth = 0i32;
                loop {
                    let l = lines[i];
                    for c in l.chars() {
                        match c {
                            '(' => depth += 1,
                            ')' => depth -= 1,
                            _ => {}
                        }
                    }
                    stmt.push_str(l.trim());
                    stmt.push(' ');
                    if depth <= 0 || i + 1 >= lines.len() {
                        break;
                    }
                    i += 1;
                }

                // The payload is everything after the format string.
                if let Some(q) = stmt.rfind('"') {
                    let after = &stmt[q + 1..];
                    let safe = after.contains("redact(")
                        || after.contains("chars()")
                        || after.contains("len()");
                    if !safe {
                        let leaked = after
                            .split(|c: char| !c.is_alphanumeric() && c != '_')
                            .any(|w| TEXT_BINDINGS.contains(&w));
                        if leaked {
                            offenders.push(format!(
                                "{}:{}: {}",
                                name,
                                start_line + 1,
                                stmt.trim()
                            ));
                        }
                    }
                }
                i += 1;
            }
        }

        assert!(
            offenders.is_empty(),
            "log call(s) format transcript-derived text without redact():\n{}",
            offenders.join("\n")
        );
    }
}
