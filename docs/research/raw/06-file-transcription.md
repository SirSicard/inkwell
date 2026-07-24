# File Transcription (Drag & Drop) - Research

## Drag & Drop (Tauri v2)
- Listen to `tauri://drag-drop` event. Payload: `{ paths: string[], position: { x, y } }`
- Also `tauri://drag-enter`, `tauri://drag-over`, `tauri://drag-leave` for UI feedback
- Rust backend has full filesystem access, no special plugin needed

## Audio Decoding: `symphonia` (pure Rust)
Covers 95% of formats without ffmpeg:

| Format | Status |
|--------|--------|
| MP3, WAV, FLAC, OGG | ✅ |
| M4A, AAC | ✅ (isomp4) |
| MP4, MOV (audio track) | ✅ (isomp4 demuxer) |
| MKV, WebM | ✅ (mkv feature) |

```toml
symphonia = { version = "0.5", features = ["mp3", "wav", "flac", "ogg", "aac", "isomp4", "mkv", "pcm", "vorbis", "alac"] }
```

Fallback for exotic formats: bundle ffmpeg as Tauri sidecar.

## Video Audio Extraction
Symphonia handles MP4, MKV, WebM, MOV containers directly. Demuxes and decodes audio track. No separate ffmpeg needed for common formats.

## Chunking Strategy: VAD-based (using existing Silero VAD)
1. Decode file to f32 PCM @ 16kHz (streaming, not all in memory)
2. Run Silero VAD to detect speech segments with timestamps
3. Group consecutive segments into ~30s chunks
4. Each chunk keeps its absolute timestamp offset for SRT
5. Feed chunks to sherpa-onnx sequentially

Why VAD > fixed-size: no mid-word cuts, natural boundaries.

Params: min_silence 300ms, speech_pad 100ms, max_chunk 30s, min_speech 250ms.

## Progress Reporting
Four phases via Tauri events:
1. `decoding` (0-15%): reading + decoding
2. `analyzing` (15-20%): VAD pass
3. `transcribing` (20-100%): per-chunk STT with live partial text
4. `complete`

## SRT Timestamps
sherpa-onnx provides token-level timestamps via `OfflineRecognizerResult.timestamps`. Combined with VAD chunk offsets, gives accurate absolute timestamps. Group tokens into subtitle lines by sentence boundary or max ~7 words.

## Memory Management
- 2-hour file @ 16kHz = ~460MB if loaded entirely
- **Two-pass approach** for large files:
  - Pass 1: stream through for VAD only (low memory)
  - Pass 2: seek to each speech segment, decode and transcribe
- For files <10 min: single-pass decode-all is fine
- Symphonia's `next_packet()` is already streaming

## Dependencies
```toml
symphonia = { version = "0.5", features = [...] }
rubato = "0.16"  # already have this for resampling
```
