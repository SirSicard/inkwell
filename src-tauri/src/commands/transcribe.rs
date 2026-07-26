//! File transcription (as opposed to live dictation, which runs in `pipeline`).

use crate::{filetranscribe, AppState};
use tauri::Emitter;

/// Transcribe an audio/video file. Decodes, runs VAD, chunks, and transcribes.
/// Emits progress events: file-transcribe-progress { phase, percent, text? }
#[tauri::command]
pub async fn transcribe_file(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<serde_json::Value, String> {
    use serde_json::json;

    let file_path = std::path::Path::new(&path);
    if !file_path.exists() {
        return Err(format!("File not found: {}", path));
    }
    if !filetranscribe::is_supported(file_path) {
        return Err(format!(
            "Unsupported format. Supported: {}",
            filetranscribe::SUPPORTED_EXTENSIONS.join(", ")
        ));
    }

    let filename = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    // Phase 1: Decode
    let _ = app.emit(
        "file-transcribe-progress",
        json!({ "phase": "decoding", "percent": 5, "filename": &filename }),
    );
    let samples = filetranscribe::decode_to_pcm(file_path)?;
    let duration_s = samples.len() as f32 / 16000.0;

    let _ = app.emit(
        "file-transcribe-progress",
        json!({ "phase": "decoding", "percent": 15, "filename": &filename }),
    );

    // Phase 2: VAD chunking
    let _ = app.emit(
        "file-transcribe-progress",
        json!({ "phase": "analyzing", "percent": 18, "filename": &filename }),
    );
    let vad_path = state.vad_model_path.lock().unwrap().clone();
    let vad_threshold = state.settings.lock().unwrap().vad_threshold;

    let chunks = if !vad_path.is_empty() && std::path::Path::new(&vad_path).exists() {
        filetranscribe::vad_chunk(&samples, &vad_path, vad_threshold)?
    } else {
        vec![(0u64, samples.clone())]
    };

    let _ = app.emit(
        "file-transcribe-progress",
        json!({ "phase": "analyzing", "percent": 20, "filename": &filename }),
    );

    // Phase 3: Transcribe each chunk.
    //
    // One request per chunk rather than one lock held for the whole file: the
    // engine thread interleaves them with hotkey dictation, so recording no
    // longer blocks until a long file finishes.

    let total_chunks = chunks.len();
    let mut segments: Vec<serde_json::Value> = Vec::new();
    let mut full_text = String::new();

    for (i, (start_ms, chunk)) in chunks.iter().enumerate() {
        let pct = 20 + ((i as f32 / total_chunks as f32) * 80.0) as u32;
        let _ = app.emit(
            "file-transcribe-progress",
            json!({
                "phase": "transcribing", "percent": pct, "chunk": i + 1,
                "total_chunks": total_chunks, "filename": &filename
            }),
        );

        match state.engine.transcribe(chunk.clone()) {
            Ok(text) if !text.is_empty() => {
                let end_ms = start_ms + (chunk.len() as u64 * 1000 / 16000);
                segments.push(json!({
                    "start_ms": start_ms,
                    "end_ms": end_ms,
                    "text": &text,
                }));
                if !full_text.is_empty() {
                    full_text.push(' ');
                }
                full_text.push_str(&text);
            }
            Ok(_) => {}
            Err(e) => log::warn!("Chunk {} transcription failed: {}", i, e),
        }
    }

    let _ = app.emit(
        "file-transcribe-progress",
        json!({ "phase": "complete", "percent": 100, "filename": &filename }),
    );

    // Apply style formatting to full text
    let current_style = state.style.lock().unwrap().clone();
    let styled = current_style.format(&full_text);

    // Apply dictionary corrections
    let styled = state.dict.with(|d| d.apply(&styled));

    // Save to transcript history
    let model_name = state.engine.name();
    let db_guard = state.db.lock().unwrap();
    if let Some(db) = db_guard.as_ref() {
        let duration_ms = (duration_s * 1000.0) as i64;
        let style_name = format!("{:?}", current_style).to_lowercase();
        let _ = db.insert(&styled, &full_text, &style_name, &model_name, duration_ms);
    }

    log::info!(
        "File transcription complete: {} ({:.1}s, {} segments, {} chars)",
        filename,
        duration_s,
        segments.len(),
        styled.len()
    );

    Ok(json!({
        "filename": filename,
        "duration_s": duration_s,
        "text": styled,
        "raw_text": full_text,
        "segments": segments,
    }))
}
