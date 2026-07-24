# Real-time Streaming Transcription - Research

## Two Approaches in sherpa-onnx

### True Streaming (OnlineRecognizer)
- Uses natively streaming models (Streaming Zipformer)
- Feeds small audio chunks (200ms) incrementally
- Gives partial/interim results that update as you speak
- Built-in endpoint detection (speech pause = commit)
- Sub-250ms latency from speech to screen
- English model: ~90MB (encoder+decoder+joiner)

### Simulated Streaming (OfflineRecognizer + VAD)
- Uses existing offline models (Parakeet, Moonshine) with Silero VAD
- VAD detects speech segments, offline model transcribes each segment
- No interim results within a segment
- Higher latency (0.5-3s per segment)
- No extra model needed (uses what we already have)
- sherpa-onnx Rust examples 39-40 show this pattern

## Recommended: Three-phase approach

### Phase 1: True Streaming (Zipformer)
- Add Streaming Zipformer English model (~90MB)
- `OnlineRecognizer` with `accept_waveform()` in a loop
- Chunk size: 3200 samples (200ms at 16kHz)
- Emit `stt:partial` (interim) and `stt:final` (committed) Tauri events
- Endpoint detection: 1.2s trailing silence = commit

### Phase 2: Hybrid Mode
- During recording: streaming Zipformer for instant preview (lower accuracy)
- On stop: run Parakeet on full audio (existing pipeline)
- Replace streaming text with higher-quality result
- User sees text immediately, then it "upgrades"

### Phase 3: Simulated Streaming (alternative)
- VAD detects speech segments in real-time
- Feed each completed segment to Parakeet/Moonshine
- Per-segment results, no interim within segments
- Simpler (single model) but higher latency

## Audio Pipeline
```
Mic (cpal) → mpsc channel → Recognition thread
                              ↓
                    chunk → accept_waveform → decode
                              ↓
                    is_endpoint? → emit partial or final
```

Chunk size tradeoffs:
- 100ms: most responsive, higher CPU
- 200ms: default, good balance
- 300ms: lower CPU, slightly delayed

## Frontend Display
- Finalized text: solid color, committed sentences
- Interim text: muted/italic, updates in real-time
- Blinking cursor during recording
- `stt:partial` updates interim, `stt:final` commits to finalized + clears interim

## Latency Breakdown
| Component | Latency |
|---|---|
| Audio chunk buffer | 100-200ms |
| Zipformer decode | 10-50ms |
| Tauri IPC | <1ms |
| React render | <16ms |
| **Total (partial)** | **~150-250ms** |
| **Total (final)** | **~1.5-2.5s** (after pause) |

## How Others Do It
- **WhisperLive**: local agreement policy (compare consecutive runs, commit agreed text)
- **Buzz**: VAD chunking, processes complete segments, no true interim
- **MacWhisper**: primarily file-based, VAD chunking for live mode

## Model
```
# Streaming Zipformer English
sherpa-onnx-streaming-zipformer-en-2023-06-26 (~90MB)
encoder.onnx + decoder.onnx + joiner.onnx + tokens.txt
```
