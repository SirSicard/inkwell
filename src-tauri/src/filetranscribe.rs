use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Decode an audio/video file to f32 mono PCM at 16kHz.
/// Uses symphonia for pure-Rust decoding of MP3, WAV, FLAC, OGG, M4A, MP4, MKV, etc.
pub fn decode_to_pcm(path: &Path) -> Result<Vec<f32>, String> {
    let file = std::fs::File::open(path)
        .map_err(|e| format!("Cannot open file: {}", e))?;

    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("Unsupported format: {}", e))?;

    let mut format = probed.format;

    // Find the first audio track
    let track = format.tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or("No audio track found")?;

    let track_id = track.id;
    let codec_params = track.codec_params.clone();
    let source_rate = codec_params.sample_rate.unwrap_or(44100) as usize;
    let channels = codec_params.channels.map(|c| c.count()).unwrap_or(1);

    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|e| format!("Codec not supported: {}", e))?;

    let mut all_samples: Vec<f32> = Vec::new();

    // Decode all packets
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(symphonia::core::errors::Error::ResetRequired) => {
                // Some formats need a reset mid-stream
                decoder.reset();
                continue;
            }
            Err(e) => return Err(format!("Decode error: {}", e)),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue, // skip corrupt frames
            Err(e) => return Err(format!("Decode error: {}", e)),
        };

        let spec = *decoded.spec();
        let num_frames = decoded.frames();
        let mut sample_buf = SampleBuffer::<f32>::new(num_frames as u64, spec);
        sample_buf.copy_interleaved_ref(decoded);
        let samples = sample_buf.samples();

        // Mix to mono
        if channels > 1 {
            for frame in samples.chunks(channels) {
                let mono: f32 = frame.iter().sum::<f32>() / channels as f32;
                all_samples.push(mono);
            }
        } else {
            all_samples.extend_from_slice(samples);
        }
    }

    if all_samples.is_empty() {
        return Err("No audio data decoded".to_string());
    }

    log::info!(
        "Decoded: {} samples ({:.1}s at {}Hz, {} ch)",
        all_samples.len(),
        all_samples.len() as f32 / source_rate as f32,
        source_rate,
        channels
    );

    // Resample to 16kHz if needed
    if source_rate != 16000 {
        let resampled = crate::recording::resample_to_16k(&all_samples, source_rate)?;
        log::info!(
            "Resampled: {} -> {} samples (16kHz, {:.1}s)",
            all_samples.len(), resampled.len(), resampled.len() as f32 / 16000.0
        );
        Ok(resampled)
    } else {
        Ok(all_samples)
    }
}

/// Chunk audio by VAD speech segments, grouped into max ~30s chunks.
/// Returns Vec of (start_ms, chunk_samples).
pub fn vad_chunk(
    samples: &[f32],
    vad_model_path: &str,
    vad_threshold: f32,
) -> Result<Vec<(u64, Vec<f32>)>, String> {
    let max_chunk_samples = 30 * 16000; // 30 seconds at 16kHz
    let min_speech_samples = (0.25 * 16000.0) as usize; // 250ms minimum

    // Run VAD to get speech regions
    let speech = crate::vad::remove_silence(samples, vad_model_path, vad_threshold)?;

    if speech.is_empty() {
        return Err("No speech detected in file".to_string());
    }

    // For now, simple fixed-size chunking of the VAD-cleaned audio.
    // Each chunk gets a timestamp based on its position.
    let mut chunks = Vec::new();
    let mut offset = 0usize;

    while offset < speech.len() {
        let end = (offset + max_chunk_samples).min(speech.len());
        let chunk = speech[offset..end].to_vec();

        if chunk.len() >= min_speech_samples {
            let start_ms = (offset as u64 * 1000) / 16000;
            chunks.push((start_ms, chunk));
        }

        offset = end;
    }

    log::info!(
        "VAD chunking: {:.1}s speech -> {} chunks",
        speech.len() as f32 / 16000.0,
        chunks.len()
    );

    Ok(chunks)
}

/// Supported file extensions for drag-and-drop
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "mp3", "wav", "flac", "ogg", "m4a", "aac",
    "mp4", "mov", "mkv", "webm", "avi", "wma",
];

pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| SUPPORTED_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}
