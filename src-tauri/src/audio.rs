use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

/// A rolling window of the most recent capture, always written, read at
/// recording start and stop. This is what makes push-to-talk forgiving:
/// people begin speaking as they press the key and keep speaking as they
/// release it, so the recording proper is bracketed with audio that only this
/// buffer still has.
///
/// Lock-free by construction rather than by cleverness: the capture callback is
/// the only writer, readers tolerate a racy snapshot (the window is 300ms out
/// of a 3s buffer, so the writer would need to lap 90% of the ring during a
/// microsecond memcpy for a read to tear), and each slot is an AtomicU32 of
/// f32 bits so no torn sample is ever read.
///
/// Its predecessor was a ringbuf::HeapRb whose consumer was dropped at
/// creation: pushes failed forever once it filled, and no pre-roll was ever
/// read. This replaces it outright.
pub struct Preroll {
    buf: Vec<AtomicU32>,
    /// Total samples ever written; monotonic. The ring position is write % len.
    write: AtomicUsize,
}

impl Preroll {
    fn new(capacity: usize) -> Self {
        let mut buf = Vec::with_capacity(capacity);
        buf.resize_with(capacity, || AtomicU32::new(0));
        Self { buf, write: AtomicUsize::new(0) }
    }

    /// Callback side. The slot store may be Relaxed, but the index store is
    /// Release so a reader that observes index w also observes every slot
    /// written before it; with both Relaxed, the newest few samples of a
    /// `last()` snapshot could still hold the previous lap's audio.
    #[inline]
    fn push(&self, sample: f32) {
        let w = self.write.load(Ordering::Relaxed);
        self.buf[w % self.buf.len()].store(sample.to_bits(), Ordering::Relaxed);
        self.write.store(w + 1, Ordering::Release);
    }

    /// Absolute position marker, for `since`.
    pub fn mark(&self) -> usize {
        self.write.load(Ordering::Acquire)
    }

    /// The last `n` samples before now, oldest first.
    pub fn last(&self, n: usize) -> Vec<f32> {
        let w = self.write.load(Ordering::Acquire);
        let n = n.min(w).min(self.buf.len());
        self.range(w - n, w)
    }

    /// Samples written after `mark`, capped at `max`, oldest first. Used to
    /// collect the release tail: mark at stop, read 300ms later.
    pub fn since(&self, mark: usize, max: usize) -> Vec<f32> {
        let w = self.write.load(Ordering::Acquire);
        let end = w.min(mark + max);
        if end <= mark || w - mark > self.buf.len() {
            // Nothing new, or the ring has already lapped the mark.
            return Vec::new();
        }
        self.range(mark, end)
    }

    fn range(&self, from: usize, to: usize) -> Vec<f32> {
        (from..to)
            .map(|i| f32::from_bits(self.buf[i % self.buf.len()].load(Ordering::Relaxed)))
            .collect()
    }
}

pub struct AudioState {
    pub rms: Arc<Mutex<f32>>,
    pub is_recording: Arc<AtomicBool>,
    /// Set while a recording is held open but not accumulating. The session
    /// stays alive and the buffer keeps what it already has; only the append
    /// stops. That is the whole of "pause", and it is why this costs one flag.
    pub is_paused: Arc<AtomicBool>,
    pub recording_buffer: Arc<Mutex<Vec<f32>>>,
    /// Rolling recent-capture window; see [`Preroll`].
    pub preroll: Arc<Preroll>,
    /// Absolute pre-roll position up to which audio was already claimed by an
    /// earlier take (its lead or its release tail). The next take's lead reads
    /// from here at the earliest, so rapid back-to-back dictations never
    /// transcribe (and paste) the same 600ms twice.
    pub preroll_claimed: Arc<AtomicUsize>,
    /// Pre-roll samples seeded into the current take, so the stop path can
    /// gate on live speech rather than on a buffer that always starts 300ms
    /// full. Without this, the too-short guard was dead and a stray hotkey tap
    /// transcribed whatever the room said before the tap.
    pub lead_len: Arc<AtomicUsize>,
    pub sample_rate: usize,
    _stream: Stream,
}

/// A selectable input device.
#[derive(serde::Serialize, Clone)]
pub struct DeviceInfo {
    /// cpal device name. This is the selection key stored in `mic_device`.
    pub id: String,
    /// Display name, possibly with the manufacturer appended.
    pub name: String,
}

fn describe(device: &cpal::Device) -> Option<DeviceInfo> {
    let desc = device.description().ok()?;
    let id = desc.name().to_string();

    // `extended()` is a WASAPI-only bag of strings; on CoreAudio it is empty,
    // so keying the display name off it hid every Mac device behind its raw
    // cpal name. Manufacturer is the one extra field the backends agree on.
    let name = match desc.manufacturer() {
        Some(m) if !m.is_empty() && !id.to_lowercase().contains(&m.to_lowercase()) => {
            format!("{} ({})", id, m)
        }
        _ => id.clone(),
    };

    Some(DeviceInfo { id, name })
}

/// List available input devices.
pub fn list_input_devices() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    let devices: Vec<DeviceInfo> = host.input_devices()
        .map(|devices| devices.filter_map(|d| describe(&d)).collect())
        .unwrap_or_default();

    for (i, d) in devices.iter().enumerate() {
        log::info!("Input device [{}]: display='{}' id='{}'", i, d.name, d.id);
    }
    devices
}

/// Resolve the `mic_device` setting to a device. "auto" (or empty) means the
/// system default input; anything else is matched against the cpal device name.
fn resolve_device(host: &cpal::Host, preferred: &str) -> Option<cpal::Device> {
    let want = preferred.trim();
    if !want.is_empty() && !want.eq_ignore_ascii_case("auto") {
        if let Ok(devices) = host.input_devices() {
            for device in devices {
                if let Ok(desc) = device.description() {
                    if desc.name().eq_ignore_ascii_case(want) {
                        log::info!("Using configured input device: {}", desc.name());
                        return Some(device);
                    }
                }
            }
        }
        log::warn!(
            "Configured mic '{}' not available, falling back to system default",
            want
        );
    }

    let device = host.default_input_device();
    if let Some(d) = &device {
        if let Ok(desc) = d.description() {
            log::info!("Using default input device: {}", desc.name());
        }
    }
    device
}

/// Start the always-on audio capture stream on the configured device.
/// `preferred_device` is the `mic_device` setting ("auto" for system default).
/// Collapse one interleaved frame to mono.
///
/// Averaging is right for mono and true stereo, but wrong for the multi-mic
/// arrays built into MacBooks: those channels are spatially separated, so the
/// same voice arrives phase-shifted on each one and summing them comb-filters
/// the speech instead of reinforcing it. For arrays, take the primary channel.
fn downmix(frame: &[f32], channels: usize) -> f32 {
    if channels > 2 {
        frame.first().copied().unwrap_or(0.0)
    } else {
        frame.iter().sum::<f32>() / channels as f32
    }
}

#[cfg(test)]
mod downmix_tests {
    use super::downmix;

    #[test]
    fn mono_passes_through() {
        assert_eq!(downmix(&[0.5], 1), 0.5);
    }

    #[test]
    fn stereo_averages() {
        assert_eq!(downmix(&[0.4, 0.6], 2), 0.5);
    }

    #[test]
    fn array_takes_the_primary_channel_instead_of_cancelling() {
        // Same voice, phase-shifted across an array: averaging would cancel it
        // to near zero, which is exactly the bug this avoids.
        assert_eq!(downmix(&[0.6, -0.6, 0.6], 3), 0.6);
    }
}

pub fn start_audio_capture(
    app_handle: AppHandle,
    preferred_device: &str,
) -> Result<AudioState, String> {
    let host = cpal::default_host();
    let device = resolve_device(&host, preferred_device)
        .ok_or("No input device found")?;

    let config = device
        .default_input_config()
        .map_err(|e| format!("No default input config: {}", e))?;

    let dev_name = device.description().map(|d| d.name().to_string()).unwrap_or_default();
    let sample_rate = config.sample_rate() as usize;
    let channels = config.channels() as usize;

    log::info!(
        "Audio device: {} ({}Hz, {} channels, {:?})",
        dev_name, sample_rate, channels, config.sample_format()
    );

    let preroll = Arc::new(Preroll::new(sample_rate * 3));
    let preroll_writer = preroll.clone();
    let preroll_writer_i16 = preroll.clone();
    let preroll_claimed = Arc::new(AtomicUsize::new(0));
    let lead_len = Arc::new(AtomicUsize::new(0));

    // Shared state
    let rms = Arc::new(Mutex::new(0.0f32));
    let rms_writer = rms.clone();

    let is_recording = Arc::new(AtomicBool::new(false));
    let is_recording_reader = is_recording.clone();

    let is_paused = Arc::new(AtomicBool::new(false));
    let is_paused_reader = is_paused.clone();

    let recording_buffer = Arc::new(Mutex::new(Vec::<f32>::new()));
    let recording_buffer_writer = recording_buffer.clone();

    // RMS smoothing
    let mut smooth_rms: f32 = 0.0;
    let mut sample_count: usize = 0;
    let mut sum_squares: f32 = 0.0;
    let emit_interval = sample_rate / 20; // 50ms

    let app_handle_clone = app_handle.clone();
    let stream_config: StreamConfig = config.clone().into();

    let app_err_handle = app_handle.clone();
    let err_fn = move |err: cpal::StreamError| {
        log::error!("Audio stream error: {}", err);
        let _ = app_err_handle.emit("mic-error", format!("{}", err));
    };

    let stream = match config.sample_format() {
        SampleFormat::F32 => device
            .build_input_stream(
                &stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let recording = is_recording_reader.load(Ordering::Relaxed)
                        && !is_paused_reader.load(Ordering::Relaxed);

                    // One lock per callback, not one per sample: the previous
                    // shape acquired the mutex for every sample on the realtime
                    // audio thread, thousands of times per callback.
                    let mut buf_guard = if recording {
                        recording_buffer_writer.lock().ok()
                    } else {
                        None
                    };

                    for frame in data.chunks(channels) {
                        let mono: f32 = downmix(frame, channels);

                        preroll_writer.push(mono);

                        if let Some(buf) = buf_guard.as_mut() {
                            buf.push(mono);
                        }

                        // RMS computation
                        sum_squares += mono * mono;
                        sample_count += 1;

                        if sample_count >= emit_interval {
                            let raw_rms = (sum_squares / sample_count as f32).sqrt();
                            // Asymmetric: snap up, ease down. A single 0.15
                            // coefficient gave a ~330ms time constant in both
                            // directions, which is why the level felt like it
                            // lagged behind the voice rather than tracking it.
                            // Fast attack makes speech onsets land immediately;
                            // the slower release keeps the meter from strobing
                            // between syllables.
                            let coeff = if raw_rms > smooth_rms { 0.55 } else { 0.18 };
                            smooth_rms += (raw_rms - smooth_rms) * coeff;

                            if let Ok(mut rms_val) = rms_writer.lock() {
                                *rms_val = smooth_rms;
                            }

                            // Only emit amplitude to frontend when recording
                            if recording {
                                let _ = app_handle_clone.emit("audio-amplitude", smooth_rms);
                            }

                            sum_squares = 0.0;
                            sample_count = 0;
                        }
                    }
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("Failed to build F32 stream: {}", e))?,

        SampleFormat::I16 => device
            .build_input_stream(
                &stream_config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let recording = is_recording_reader.load(Ordering::Relaxed)
                        && !is_paused_reader.load(Ordering::Relaxed);

                    let mut buf_guard = if recording {
                        recording_buffer_writer.lock().ok()
                    } else {
                        None
                    };

                    for frame in data.chunks(channels) {
                        let mono: f32 = if channels > 2 {
                            frame.first().map(|&s| s as f32 / 32768.0).unwrap_or(0.0)
                        } else {
                            frame.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / channels as f32
                        };

                        preroll_writer_i16.push(mono);

                        if let Some(buf) = buf_guard.as_mut() {
                            buf.push(mono);
                        }

                        sum_squares += mono * mono;
                        sample_count += 1;

                        if sample_count >= emit_interval {
                            let raw_rms = (sum_squares / sample_count as f32).sqrt();
                            // Asymmetric: snap up, ease down. A single 0.15
                            // coefficient gave a ~330ms time constant in both
                            // directions, which is why the level felt like it
                            // lagged behind the voice rather than tracking it.
                            // Fast attack makes speech onsets land immediately;
                            // the slower release keeps the meter from strobing
                            // between syllables.
                            let coeff = if raw_rms > smooth_rms { 0.55 } else { 0.18 };
                            smooth_rms += (raw_rms - smooth_rms) * coeff;

                            if let Ok(mut rms_val) = rms_writer.lock() {
                                *rms_val = smooth_rms;
                            }

                            // Only emit amplitude to frontend when recording
                            if recording {
                                let _ = app_handle_clone.emit("audio-amplitude", smooth_rms);
                            }

                            sum_squares = 0.0;
                            sample_count = 0;
                        }
                    }
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("Failed to build I16 stream: {}", e))?,

        format => return Err(format!("Unsupported sample format: {:?}", format)),
    };

    stream.play().map_err(|e| format!("Failed to start stream: {}", e))?;
    log::info!("Audio capture started (always-on, {}s pre-roll window)", 3);

    Ok(AudioState {
        rms,
        is_recording,
        is_paused,
        recording_buffer,
        preroll,
        preroll_claimed,
        lead_len,
        sample_rate,
        _stream: stream,
    })
}

/// Tear down the capture stream and reopen it on `preferred_device`.
/// Call this whenever the `mic_device` setting changes, because the stream is bound
/// to one device at build time, so nothing else makes the setting take effect.
pub fn restart_audio_capture(app: &AppHandle, preferred_device: &str) -> Result<(), String> {
    let state = app.state::<crate::AppState>();
    let mut guard = state
        .audio
        .lock()
        .map_err(|_| "Audio state lock poisoned".to_string())?;

    if let Some(existing) = guard.as_ref() {
        if existing.is_recording.load(Ordering::Relaxed) {
            return Err("Cannot switch microphone while recording".to_string());
        }
    }

    // Drop the old stream before opening the new one: backends refuse a second
    // capture stream on a device the first one still holds.
    *guard = None;

    let new_state = match start_audio_capture(app.clone(), preferred_device) {
        Ok(s) => s,
        Err(e) => {
            // The old stream is already gone, so bailing here would leave the
            // app with no mic at all. Fall back to the default device and say so.
            log::error!("Mic switch to '{}' failed: {}", preferred_device, e);
            let _ = app.emit(
                "mic-error",
                format!("Could not open '{}' ({}). Falling back to the default microphone.", preferred_device, e),
            );
            let recovered = start_audio_capture(app.clone(), "auto")
                .map_err(|e2| format!("{} (default device also failed: {})", e, e2))?;
            *guard = Some(recovered);
            return Err(e);
        }
    };
    *guard = Some(new_state);
    // A deliberate reopen counts as use: without this, an old timestamp could
    // hand the just-opened stream straight back to the idle watchdog.
    *state.mic_last_used.lock().unwrap() = std::time::Instant::now();
    log::info!("Audio capture restarted on '{}'", preferred_device);
    Ok(())
}

#[tauri::command]
pub fn get_input_devices() -> Vec<DeviceInfo> {
    list_input_devices()
}

#[cfg(test)]
mod preroll_tests {
    use super::Preroll;

    #[test]
    fn last_returns_most_recent_samples_in_order() {
        let p = Preroll::new(8);
        for i in 0..5 {
            p.push(i as f32);
        }
        assert_eq!(p.last(3), vec![2.0, 3.0, 4.0]);
    }

    #[test]
    fn last_survives_wraparound() {
        let p = Preroll::new(4);
        for i in 0..10 {
            p.push(i as f32);
        }
        assert_eq!(p.last(3), vec![7.0, 8.0, 9.0]);
        // Asking for more than the ring holds returns what exists.
        assert_eq!(p.last(100), vec![6.0, 7.0, 8.0, 9.0]);
    }

    #[test]
    fn last_on_an_empty_ring_is_empty() {
        let p = Preroll::new(4);
        assert!(p.last(3).is_empty());
    }

    #[test]
    fn since_returns_only_what_came_after_the_mark() {
        let p = Preroll::new(8);
        p.push(1.0);
        let mark = p.mark();
        p.push(2.0);
        p.push(3.0);
        assert_eq!(p.since(mark, 10), vec![2.0, 3.0]);
        // Capped at max.
        assert_eq!(p.since(mark, 1), vec![2.0]);
    }

    #[test]
    fn since_a_lapped_mark_returns_nothing_rather_than_garbage() {
        let p = Preroll::new(4);
        let mark = p.mark();
        for i in 0..9 {
            p.push(i as f32);
        }
        // The ring wrapped past the mark; the honest answer is nothing.
        assert!(p.since(mark, 4).is_empty());
    }

    #[test]
    fn since_with_no_new_samples_is_empty() {
        let p = Preroll::new(4);
        p.push(1.0);
        let mark = p.mark();
        assert!(p.since(mark, 10).is_empty());
    }
}
