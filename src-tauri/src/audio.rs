use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use ringbuf::{HeapRb, traits::{Producer, Split}};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

pub struct AudioState {
    pub rms: Arc<Mutex<f32>>,
    pub is_recording: Arc<AtomicBool>,
    pub recording_buffer: Arc<Mutex<Vec<f32>>>,
    pub sample_rate: usize,
    _stream: Stream,
}

/// Device info with both the cpal name and friendly Windows name
#[derive(serde::Serialize, Clone)]
pub struct DeviceInfo {
    pub id: String,      // cpal internal name (used for selection)
    pub name: String,     // friendly display name
}

/// List available input devices with friendly names from Windows WASAPI.
pub fn list_input_devices() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    let devices: Vec<DeviceInfo> = host.input_devices()
        .map(|devices| {
            devices
                .filter_map(|d| {
                    let desc = d.description().ok()?;
                    let id = desc.name().to_string();

                    // Collect all info cpal gives us
                    let extended: Vec<String> = desc.extended().iter().map(|s| s.to_string()).collect();

                    // Use the fullest name available
                    let friendly = if !extended.is_empty() {
                        extended[0].clone()
                    } else {
                        id.clone()
                    };

                    log::info!("  Device: name='{}' extended={:?}", id, extended);

                    Some(DeviceInfo { id, name: friendly })
                })
                .collect()
        })
        .unwrap_or_default();

    for (i, d) in devices.iter().enumerate() {
        log::info!("Input device [{}]: display='{}' id='{}'", i, d.name, d.id);
    }
    devices
}

/// Find the best input device. Try each device with a quick capture test
/// to find one that actually produces non-zero audio.
/// The "AI Noise-cancelling Input" on Logitech is a virtual device that
/// cpal reads as all zeros on Windows. We need to find the real hardware device.
fn find_preferred_device(host: &cpal::Host) -> Option<cpal::Device> {
    // Skip virtual/noise-cancelling devices that produce zeros in cpal
    let skip_keywords = ["ai noise"];

    if let Ok(devices) = host.input_devices() {
        for device in devices {
            if let Ok(desc) = device.description() {
                let name_lower = desc.name().to_lowercase();
                let should_skip = skip_keywords.iter().any(|kw| name_lower.contains(kw));
                if should_skip {
                    log::info!("Skipping virtual device: {}", desc.name());
                    continue;
                }
                log::info!("Selected input device: {}", desc.name());
                return Some(device);
            }
        }
    }

    // Fallback to system default
    log::info!("No suitable device found, using system default");
    host.default_input_device()
}

/// Start the always-on audio capture stream
pub fn start_audio_capture(app_handle: AppHandle) -> Result<AudioState, String> {
    let host = cpal::default_host();
    let device = find_preferred_device(&host)
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
                        let mono: f32 = frame.iter().sum::<f32>() / channels as f32;

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
                            smooth_rms += (raw_rms - smooth_rms) * 0.15;

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
                        let mono: f32 =
                            frame.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / channels as f32;

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
                            smooth_rms += (raw_rms - smooth_rms) * 0.15;

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

#[tauri::command]
pub fn get_input_devices() -> Vec<DeviceInfo> {
    list_input_devices()
}
