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
   replaces it with revision 2 in one transaction, and refuses an empty result or one in which any
   channel falls below half its previous words (per channel, so a healthy far end cannot hide a mic
   pass that came back empty).
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
| `ink-shader` | Build-time only: turns `shaders/ink.wgsl` into the Metal source the app compiles at runtime (naga as a library); a test fails when the checked-in MSL is stale. Never linked into the app. |
| `ink-bench` | Replay plus WER, DER, ERLE and latency. Reads `$INK_BENCH_DIR`. |

Later: `mac/` (Swift package: app, Apple engines, renderer, the core as an XCFramework),
`windows/` (the WinUI 3 solution), `shaders/ink.wgsl` (one shader, translated by naga to MSL and
HLSL), `schema/events.schema.json`, and `fixtures/` (synthetic and public-licensed audio only).

### ggml: two copies, kept apart

Two engines bring their own ggml, the tensor library under llama.cpp: llama.cpp (Qwen3-ASR and
local chat models, the `engine-llama` feature) and NeMo-Speech.cpp (the diarizer). The versions
are far apart: llama-cpp-sys-2 0.1.157 vendors ggml 0.24.0, and NeMo-Speech.cpp ships 0.12 as its
own `libggml*` libraries. So they are **not** built against one shared ggml: each engine keeps the
ggml it was measured with, and a llama.cpp update never drags the diarizer along.

- llama.cpp and its ggml are linked **statically** into the core. The core loads no llama or ggml
  library, so nothing of ours can collide with the diarizer's `libggml*` at load time.
- The diarizer stays in its own shared library beside its own ggml, bound per library (the
  two-level namespace on macOS, per-DLL imports on Windows). Its adapter calls only the diarizer's
  C API and never links ggml by name: `-lggml` would bind to whichever ggml the linker meets first.
- `core/crates/ink-engines/tests/ggml_link.rs` checks this on the linked binary: no engine library
  is loaded, no `ggml_*`, `llama_*` or `mtmd_*` symbol is imported, llama.cpp's are all defined in
  the core, and the ggml the core calls reports llama.cpp's version. With the diarizer's feature on,
  it also shows the diarizer's library importing ggml from its own copy, and both copies answering
  in one process.

The cost is a second copy of ggml's code and a second Metal device setup.

Both copies are built for every Apple silicon Mac, not for the one that built them:
`GGML_NATIVE=OFF` and no `-march`, so ggml gets the compiler's arm64 macOS default, the M1's
instruction set (NEON, dot product, FP16 arithmetic; no int8 matrix multiply, SVE or SME). A native
build on a newer Mac compiles in i8mm and SME kernels that stop an older one with an illegal
instruction, and nothing on the building Mac shows it. llama-cpp-sys-2 builds non-native unless
`RUSTFLAGS` carries `target-cpu=native`; `mac/scripts/build-core.sh` reads the value back from
llama.cpp's CMake cache and fails on anything else. The diarizer's prefix records its own (below).
(The pure-Rust VAD runtime, tract, carries SME kernels too, but chooses them at run time from what
the CPU reports.)

## The diarizer's native library

The diarizer (Nemotron-3-Diarization) runs on NeMo-Speech.cpp, a C++ library with a C API. The
VAD (Silero) needs no native code: it runs on tract, a pure-Rust ONNX runtime.

- **Built outside cargo, not vendored.** `core/crates/ink-engines/native/build-nemo-speech.sh`
  builds NeMo-Speech.cpp at its pinned commit from a checkout (the upstream `metal-diar` preset and
  upstream's own ggml patch step) and installs it into a prefix; `NEMO_SPEECH_DIR` names that
  prefix when building with `--features engine-nemo`. There is no submodule and no copy of the
  sources: the repository carries no C++ and no machine paths, and cargo never runs CMake or
  touches the network.
- **Checked before it is linked.** `build.rs` compares the installed headers with the pinned
  commit's (the adapter declares the C ABI by hand) and checks every installed library against
  the manifest the build script wrote (the commit and each library's SHA-256).
- **In CI** the adapter is compiled and linted without the library (`INK_NEMO_CHECK_ONLY=1`); the
  tests that run it, and reproduce the diarizer's DER on AMI, run locally.
- **Built to ship.** The script builds with `GGML_NATIVE=OFF` (read back from the CMake cache and
  recorded in the manifest, which `build.rs` refuses without it) and for macOS 26
  (`MACOSX_DEPLOYMENT_TARGET`, Package.swift's platform). Its prefix is self-contained: every
  library NeMo-Speech.cpp loads from outside the OS (SentencePiece and Abseil) is copied into
  `<prefix>/lib`, every library there is loaded by `@rpath` with `@loader_path` as its only rpath,
  and the script checks that nothing in the prefix names an absolute path outside `/usr/lib` and
  `/System/Library` before it writes the manifest. The manifest also names where each copy came
  from, for its licence.
- **Found at run time by rpath, never by `DYLD_LIBRARY_PATH`.** Test binaries: ink-engines
  declares `links = "nemo_speech_asr_c"` and hands the library's directory to dependents' build
  scripts; ink-engines gives its own binaries the rpath, and ink-pipeline's `build.rs` gives its
  test binaries the same. The core's static library carries no rpath: `build-core.sh` writes the
  linker arguments it needs (`mac/build/InkCore.link`: the prefix's `lib` and rustc's
  native-static-libs), and `build-mac.sh --engines` links the app with them, copies
  `libnemo_speech_asr_c` and everything it loads through `@rpath` into `Contents/Frameworks`, and
  gives the app binary an rpath there (`@executable_path/../Frameworks`). Each copy is signed with
  the app's identity and the hardened runtime, and carries no entitlements.
- **Checked in the bundle.** `build-mac.sh` reads every Mach-O's load commands
  (`mac/scripts/lib/bundle-check.sh`) and fails the build if any loads a path outside the bundle
  and the OS, has an rpath that is not relative to itself or the executable, cannot resolve a
  library it loads inside the bundle, or was built for a newer macOS than 26. A Homebrew built for
  a newer macOS than the app's (on a Mac running one) fails that last check; `INK_ALLOW_NEWER_MACOS=1`
  lets a local build through with a warning, and a release (`--timestamp`) refuses it. Every
  bundled library must also be one `THIRD_PARTY.md` covers, by name.
- **Its ggml stays its own**, apart from llama.cpp's static copy: see "ggml: two copies, kept
  apart" above. Linux is not a target; if it becomes one, its flat namespace would let one copy's
  symbols stand in for the other's, and the llama.cpp adapter's ggml must then hide its symbols.
- **Its dependencies**, SentencePiece and Abseil, are listed in [THIRD_PARTY.md](../THIRD_PARTY.md)
  with NeMo-Speech.cpp and its ggml; `cargo deny` cannot see them. They ship inside the app now,
  so their notices go in its About screen. They come from Homebrew at build time: Homebrew's
  Abseil has no static libraries, and its SentencePiece archive was built against a different
  Abseil than the one it installs beside it, so they are bundled as the shared libraries Homebrew
  built (SentencePiece's carries its own Abseil inside). Their versions are whatever Homebrew
  serves when the release is built; the build manifest published with each release records them.

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

## Engines the shell registers (ABI 2)

The Mac shell's Apple engines live in Swift (rule 2) and register into the core through
`ink_register_engine` ([`inkwell.h`](../core/crates/ink-ffi/include/inkwell.h) has the contract).
Each table is one kind:

| Kind | Fills | The shell's functions | Deadline per call |
|---|---|---|---|
| `INK_ENGINE_OFFLINE` | dictation and meeting finals | `transcribe` | none: the job's cancel token and shutdown end the wait |
| `INK_ENGINE_STREAMING` | live partials | `stream_open`, `stream_push`, `stream_finish`, `stream_close` | open 10 s, push 2 s, finish 30 s |
| `INK_ENGINE_LLM` | dictation polish | `generate` | 120 s |

- Every call is answered once through `ink_engine_complete`, from any thread; a live stream's
  words come back through `ink_stream_event`, which only queues. A call past its deadline is given
  up (the engine is told through `cancel`, and a late answer is refused): a stuck shell engine
  costs its stream or its take, never the meeting's worker or shutdown. Every opened stream is
  closed exactly once, and nothing is taken from it afterwards.
- Errors are a kind and a code, never text (I5). `unavailable` is how an engine says it cannot run
  on this Mac now; the core then goes on without it and never makes up a result.
- The router treats registered engines like installed registry models: the lowest measured error
  rate per job wins, at every call. So Parakeet, registered for the finals with rates worse than
  Qwen3-ASR's, serves them only while Qwen3-ASR is not installed. `engine.route` tells the shell
  what serves a job now.
- Language models are kept by the core apart from the router (whose jobs are speech jobs); dictation
  polish goes to the one registered. Foundation Models is registered only while Apple Intelligence
  is available.
- A table the size of ABI 1's still registers an offline engine; newer kinds need the full table.

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
