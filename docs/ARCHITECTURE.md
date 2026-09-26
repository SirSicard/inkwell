# Architecture

Inkwell 1.0 is one local voice app with two jobs: **dictation** (hold a key, speak, the text lands in
the focused app) and **meeting notes** (your mic and the far end as two streams, a live transcript, a
final pass, and a record with a summary and commitments). It is native on each OS, with SwiftUI and
AppKit on the Mac and WinUI 3 on Windows, over one Rust core. Everything runs on the machine unless
the user brings an API key.

Status: being built. `main` still ships the Tauri app (0.2.x) until 1.0 replaces it; 0.2 is frozen
on the `legacy/0.2` branch, and its architecture is in [legacy/ARCHITECTURE-0.2.md](legacy/ARCHITECTURE-0.2.md).

```
            Mac shell (Swift)                         Windows shell (C#)
   SwiftUI · AppKit · Metal ink · Apple engines   WinUI 3 · D3D11 ink
                  │  inkwell.h (C ABI): commands in, events out, engines registered  │
                  └──────────────────────────────┬───────────────────────────────────┘
                                          Rust core (core/)
   capture ─► ring ─► pump ─► chunk store (disk) ─► pipeline ─► gain ─► engines ─► store
                                                        └──► echo (mic only, when there is echo)
```

## Rules

1. **The core owns time.** Every realtime or latency-critical path is Rust: capture, the ring
   buffer, VAD, echo cancellation, the engines, the hotkey state machine and text insertion. Shells
   render state and send commands.
2. **Apple accelerators stay in Swift.** FluidAudio (live partials) and Foundation Models (polish)
   live in the Mac shell and register into the core as engines over the C ABI. The core never
   wraps CoreML.
3. **Disk is the seam.** Capture writes raw PCM chunks, and everything downstream reads chunks. RAM
   holds seconds, never a session.
4. **Partials are ephemeral; finals persist.** The live pass writes revision 1. The offline pass
   replaces it with revision 2 in one transaction, and refuses an empty result or one with fewer than
   half the previous words.
5. **Me versus them is stream identity.** Mic is you, the far end is them. Only the far end is
   diarized, and speaker labels are kept only when there are at least two substantial clusters,
   each holding at least 2 % of the speech.
6. **Local-only is structural.** A switch refuses any non-loopback language-model endpoint in code.
7. **One replay harness.** `FileReplaySource` drives the whole pipeline from WAV fixtures,
   identically on macOS and Windows.
8. **One event schema.** Events are defined once in `schema/`, and the Swift and C# types are
   generated from it.
9. **Draw nothing when idle.** The ink renders only while something is live; idle is a still frame.
   No polling timers.
10. **Engines per job**, chosen by measurement (word error rate on public human-labelled sets:
    AMI meetings and FLEURS English). The models are listed in [MODEL-WEIGHTS.md](MODEL-WEIGHTS.md).
    - Dictation final and meeting final: Qwen3-ASR 1.7B via llama.cpp (Metal on the Mac, Vulkan or
      CPU on Windows). One resident model serves both.
    - Live partials: Parakeet TDT v3, via FluidAudio on the Mac and sherpa-onnx on Windows.
    - Far-end diarization: Nemotron-3-Diarization via NeMo-Speech.cpp.
11. **A gain stage before every engine.** A per-utterance robust-peak gain for dictation and file
    import, and a slow AGC for meetings. Quiet speech otherwise comes back as empty text: Parakeet
    returns nothing from about −70 dBFS RMS.

**Echo.** Echo cancellation (AEC3) runs on the mic only, and only when an echo path is found (with
headphones there is none, and running it anyway deletes words). The "you" transcript reads AEC3's
linear output behind a gate that drops words where the full output says echo only: in measured
double talk the full output cost about 10 WER points and the linear output under 1. With Bluetooth
output, the built-in mic is recorded, because a headset mic is 16 kHz call audio.

## Crates

All crates exist from the first commit, so work in parallel only ever touches its own crate.

| Crate | Holds |
|---|---|
| `ink-core` | Types and **every** trait, with its threading contract. No dependencies. Mocks behind the `mock` feature. |
| `ink-audio` | Ring, chunk store, replay, resampler, downmix, gain stage, VAD, bands. |
| `ink-echo` | AEC3 wrapper, drift alignment, duplicate-line suppression. |
| `ink-store` | SQLite (WAL, STRICT, FTS5), migrations, supersede, importers. |
| `ink-engines` | Registry, downloader, router, residency; adapters behind cargo features. |
| `ink-llm` | BYOK client, local-only guard, keyring, local summaries, commitments. |
| `ink-pipeline` | Dictation chain, meeting chain, file import, watchdog. |
| `ink-platform-mac` | objc2 implementations of the platform traits. |
| `ink-platform-win` | windows-rs and WASAPI implementations of the platform traits. |
| `ink-ffi` | The C ABI (`include/inkwell.h`), event bridge, bands copy-out. |
| `ink-bench` | Replay plus WER, DER, ERLE and latency. Reads `$INK_BENCH_DIR`. |

Later: `mac/` (Swift package: app, Apple engines, renderer, the core as an XCFramework),
`windows/` (the WinUI 3 solution), `shaders/ink.wgsl` (one shader, translated by naga to MSL and
HLSL), `schema/events.schema.json`, and `fixtures/` (synthetic and public-licensed audio only).

## Threads

Every trait method in `ink-core` names the thread it may run on
([`threading.rs`](../core/crates/ink-core/src/threading.rs) has the full definitions):

- **realtime**: OS audio callbacks. No allocation, locks, blocking or logging; only the capture
  sink and the clock run here. A thread-scoped no-alloc guard enforces it in tests.
- **pump**: drains the capture rings into the chunk store; bounded work, never waits.
- **worker**: engines, the store, platform calls, language models; may block, takes a cancel token
  when long.
- **callback**: threads the platform or an external engine owns; whatever the core hands over there
  only enqueues.
- **main**: the shell's UI thread. The core never calls into it; platform code that needs it hops
  there itself.

The C ABI keeps the same shape: events reach the shell on one core thread (the shell hops to its
main thread), and engines registered from Swift are called from worker threads.

## Testing

- `cargo test --workspace` in `core/`. CI runs it with fmt, `clippy -D warnings` and `cargo deny` on
  macOS 26 and Windows Server 2025 (`.github/workflows/core.yml`).
- CI has no models, GPU, Neural Engine or audio devices. Tests there use the mock engine (answers
  keyed by the exact input audio) and the replay harness.
- Tests that need real models are `#[ignore]` and run locally:
  `INK_BENCH_DIR=<bench data> cargo test -- --ignored`.
- Capture, permission, paste and hotkey behaviour is checked by hand from checklists, since an
  agent's shell cannot hold the OS permissions they need.

## Invariants

Checked after every change once the step that introduces them has landed:

| | Invariant | Check |
|---|---|---|
| I1 | Core tests green on macOS and Windows | `core.yml` |
| I2 | Licences clean | `cargo deny`; Swift and NuGet audits once those projects exist |
| I3 | No private data in the repo | a local pre-push check |
| I4 | Realtime callbacks allocation-free | a thread-scoped guard around every audio callback |
| I5 | No transcripts in logs | a privacy lint test |
| I6 | The legacy app is untouched until 1.0 | no commits to `src/` or `src-tauri/` |
| I7 | Shell budget as shipped | idle: 0 frames and < 0.1 % CPU over 2 min; live at 60 fps: ≤ 6 % p50 and ≤ 10 % p95 of one core; memory ≤ 150 MB plus the model |
