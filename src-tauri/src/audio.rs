use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use ringbuf::{HeapRb, traits::{Producer, Split}};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

pub struct AudioState {
    pub rms: Arc<Mutex<f32>>,
    pub is_recording: Arc<AtomicBool>,
    pub recording_buffer: Arc<Mutex<Vec<f32>>>,
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

    // Ring buffer: 3 seconds standby
    let rb_size = sample_rate * 3;
    let rb = HeapRb::<f32>::new(rb_size);
    let (mut producer, _consumer) = rb.split();

    // Shared state
    let rms = Arc::new(Mutex::new(0.0f32));
    let rms_writer = rms.clone();

    let is_recording = Arc::new(AtomicBool::new(false));
    let is_recording_reader = is_recording.clone();

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
                    let recording = is_recording_reader.load(Ordering::Relaxed);

                    for frame in data.chunks(channels) {
                        let mono: f32 = downmix(frame, channels);

                        // Always push to ring buffer (standby)
                        let _ = producer.try_push(mono);

                        // If recording, also push to recording buffer
                        if recording {
                            if let Ok(mut buf) = recording_buffer_writer.lock() {
                                buf.push(mono);
                            }
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
                    let recording = is_recording_reader.load(Ordering::Relaxed);

                    for frame in data.chunks(channels) {
                        let mono: f32 = if channels > 2 {
                            frame.first().map(|&s| s as f32 / 32768.0).unwrap_or(0.0)
                        } else {
                            frame.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / channels as f32
                        };

                        let _ = producer.try_push(mono);

                        if recording {
                            if let Ok(mut buf) = recording_buffer_writer.lock() {
                                buf.push(mono);
                            }
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
    log::info!("Audio capture started (always-on, ring buffer {}s)", 3);

    Ok(AudioState {
        rms,
        is_recording,
        recording_buffer,
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
    log::info!("Audio capture restarted on '{}'", preferred_device);
    Ok(())
}

#[tauri::command]
pub fn get_input_devices() -> Vec<DeviceInfo> {
    list_input_devices()
}
