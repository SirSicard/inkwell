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
   The core holds it (`llm.local_only`, on unless turned off, on when unreadable) and every
   language-model call goes through it: dictation polish, meeting summaries and commitments, Ask.
7. **One replay harness.** `FileReplaySource` drives the whole pipeline from WAV fixtures,
   identically on macOS and Windows.
8. **One event schema.** Events are defined once in `schema/`, and the Swift and C# types are
   generated from it.
9. **Draw nothing when idle.** The ink renders only while something is live; idle is a still frame.
   No polling timers. The live icon's recording pulse is a state animation, not polling: it runs
   only while a meeting records and someone can see the screen, and stops the moment either ends.
10. **Engines per job**, chosen by measurement (word error rate on public human-labelled sets:
    AMI meetings and FLEURS English). The models are listed in [MODEL-WEIGHTS.md](MODEL-WEIGHTS.md).
    - Dictation final and meeting final: Qwen3-ASR 1.7B via llama.cpp (Metal on the Mac, Vulkan or
      CPU on Windows). One resident model serves both. On a Windows PC without a GPU, dictation
      goes to Parakeet instead (below).
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
| `ink-shader` | Build-time only: turns `shaders/ink.wgsl` into the Metal source and the HLSL (shader model 5.0, Direct3D 11) the shells compile at runtime (naga as a library), and `design/tokens.json` into the shells' design tokens (`ink-tokens`: Swift, XAML and C#); a test fails when any checked-in file is stale. Never linked into the app. |
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

### Where llama.cpp computes

The llama.cpp adapter picks its device once per process from the devices ggml reports
(`ink-engines/src/compute.rs`): the first GPU (Metal on the Mac; Vulkan on Windows, built with
`engine-llama-vulkan`), else the CPU. On a GPU every layer is offloaded and llama.cpp's own thread
defaults stand, as measured. On the CPU nothing is offloaded and the thread count is set to the
machine's physical cores; llama.cpp's default of 4 leaves most of a desktop's cores idle while the
model reads the audio. So one Windows build runs on a Vulkan GPU where there is one and on the CPU
where there is none. The CPU is a fallback, not a peer: it fits a meeting's final pass, but a
dictation takes seconds.

- The Vulkan build needs `vulkan-1.dll`, which GPU drivers install, so the core's DLL delay-loads
  it: the DLL loads on a machine without it, and when ggml registers its Vulkan backend a
  delay-load hook answers the missing loader so that Vulkan fails to start the ordinary way and
  llama.cpp runs on the CPU (`ink-engines/src/llama/no_vulkan.rs`; `tests/vulkan_missing.rs`).
  `windows/scripts/build-core.ps1` builds the release's DLL and checks its imports.
- Building it needs the Vulkan SDK, libclang (llama-cpp-sys-2 generates its bindings) and the
  Ninja generator: see the feature's note in `ink-engines/Cargo.toml`.

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
  library it loads inside the bundle, or was built for a newer macOS than 26. A local build with
  Homebrew's SentencePiece and Abseil (built for a Mac's own, newer macOS) fails that last check; `INK_ALLOW_NEWER_MACOS=1`
  lets a local build through with a warning, and a release (`--timestamp`) refuses it. Every
  bundled library must also be one `THIRD_PARTY.md` covers, by name.
- **On Windows** the same two scripts run in Git Bash inside a Visual Studio developer environment.
  NeMo-Speech.cpp builds with the upstream `vulkan-diar` preset (the dynamic C runtime). The
  developer environment's architecture picks the compiler for all three libraries
  (`native/lib/windows-toolchain.sh`): MSVC on x64, clang-cl on ARM64, because ggml's CPU backend
  refuses MSVC on ARM. The ARM64 build must run on an ARM64 machine and needs the Vulkan SDK for
  Windows on ARM64. It has not run yet: CI type-checks the adapter only.
  SentencePiece and Abseil are built from the same pinned tarballs as static libraries and linked
  into NeMo's own DLL, so the prefix's `bin/` holds only NeMo's DLLs and its ggml's. The Visual C++
  runtime is the system's. The manifest hashes those DLLs and the C API's import library in
  `lib/`. With no rpath on Windows, `build.rs` copies the checked DLLs into its `OUT_DIR` and
  declares that directory as a native search path, which cargo puts on `PATH` for tests. The
  diarizer runs on GPU 0 (Vulkan) and, where that does not load, on the CPU (about ten times
  slower: 23× real time on a 12-core desktop, not a measurement, as other builds shared it). Its
  Vulkan backend loads `vulkan-1.dll` as soon as it loads, so the core's DLL delay-loads
  `nemo_speech_asr_c.dll` and the adapter loads it before its first call (`src/nemo.rs`): on a PC
  without a Vulkan driver the diarizer is unavailable and everything else runs. The release ships
  the prefix's DLLs beside `Inkwell.exe` (`windows/scripts/build-core.ps1`).
- **Its ggml stays its own**, apart from llama.cpp's static copy: see "ggml: two copies, kept
  apart" above. Linux is not a target; if it becomes one, its flat namespace would let one copy's
  symbols stand in for the other's, and the llama.cpp adapter's ggml must then hide its symbols.
- **Its dependencies**, SentencePiece and Abseil, are listed in [THIRD_PARTY.md](../THIRD_PARTY.md)
  with NeMo-Speech.cpp and its ggml; `cargo deny` cannot see them. They ship inside the app now,
  so their notices go in its About screen. They are pinned: `native/build-sentencepiece-abseil.sh`
  builds SentencePiece 0.2.2 and Abseil 20260817.0 from release tarballs whose SHA-256s it holds,
  SentencePiece against that Abseil (its `sentencepiece_processor.h` passes `absl::Status` across
  the library boundary, so NeMo-Speech.cpp and SentencePiece must be compiled against one Abseil),
  as shared libraries installed by `@rpath`; `build-nemo-speech.sh` builds against them when
  `ENGINE_DEPS_DIR` names their prefix, as the release does. The build manifest published with each
  release records their versions and hashes.

## Parakeet on Windows (sherpa-onnx)

On the Mac, Parakeet runs in FluidAudio on the Neural Engine. On Windows the core runs it itself:
the int8 ONNX conversion of the same weights, on sherpa-onnx's C API, on the CPU (`engine-sherpa`,
`ink-engines/src/sherpa.rs`), as a registry model.

- **Not built here.** `SHERPA_ONNX_DIR` names sherpa-onnx 1.13.4's prebuilt "shared, MD, Release,
  no-tts" archive, unpacked; `build.rs` checks each file it uses against its SHA-256 before linking.
  The no-tts build carries no espeak-ng (GPL-3.0). The sherpa-onnx crates are not used: their build
  script downloads archives. In CI the adapter is type-checked without the libraries
  (`INK_SHERPA_CHECK_ONLY=1`); the real-model tests run locally.
- **Its DLLs sit beside the executable.** Windows 11 has its own, older `onnxruntime.dll` in
  System32, which Windows finds before anything on `PATH`, and sherpa-onnx given that one crashes
  the process. So the app ships `sherpa-onnx-c-api.dll`, `onnxruntime.dll` and
  `onnxruntime_providers_shared.dll` beside its executable, `build.rs` copies them beside the test
  and binary executables too, and the adapter refuses to load (with an error that says why) when
  the ONNX Runtime the process loaded is not the archive's.
- **Live partials**: [`TrailingWindow`](../core/crates/ink-engines/src/live.rs) re-decodes the
  utterance not yet settled every half second, the Mac's scheme ported (hide the newest 0.16 s;
  settle on a 0.8 s pause or at 12 s). Each window goes through the router and residency like any
  job, so live partials and dictation share one loaded copy. A still window (nothing pending, a
  stationary speech band: silence, room tone, hum) is not decoded, so a quiet side costs no CPU.
- **Dictation on a PC without a GPU.** There Qwen3-ASR takes 1.6-1.9 s for 5 s of speech, and
  Parakeet about 0.3 s (not a measurement: other builds shared the machine), at 7.2 % WER on FLEURS
  (level-normalised) against Qwen3-ASR's 4.3 %. So on such a machine the router gives dictation to
  Parakeet first, whatever the error rates (`Router::with_gpu_probe`,
  `Runtime::slow_on_cpu_for_dictation`); with a GPU, Qwen3-ASR dictates. A take longer than 90 s
  is cut into windows, as Qwen3-ASR's are.

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
- Polish sends a dictation, voice edit the selection and the instruction, and a meeting's summary
  (with its commitments) and Ask the meeting's transcript, to that model, so each runs only with
  the user's consent for where it goes: this machine, or one named cloud provider
  (`ink_pipeline::consent`, one consent per feature). The core keeps each consent
  (`llm.consent.polish`, `llm.consent.edit`, `llm.consent.meetings`); `consent.allow` records it,
  for the destination the model has at that moment, and turns the feature on (polish's switch,
  edit's key, the meetings switch) in the same write; turning the feature off withdraws it in the
  same write. Each call checks the consent against the model that call reaches
  (`Llm::complete_if`), so a model that moved from this machine to a cloud provider, or between
  providers, gets nothing until the user agrees again: a polish goes in as said with
  `polish_not_allowed`, an edit changes nothing (`not_allowed`), a meeting finishes with no
  summary or commitments (`summary_not_allowed`), and Ask answers that it needs the user's OK. The
  meeting's consent is read when the summary is written, not when the meeting starts.
- A table the size of ABI 1's still registers an offline engine; newer kinds need the full table.

## The screens' commands

The screens read and change the library and the permissions through commands too
([`inkwell.h`](../core/crates/ink-ffi/include/inkwell.h) lists them): permission checks and
requests, the open commitments ("owed"), a live meeting's notes, the model catalogue, the user's
modes, each language-model feature's state and consent (`consent.get`, `consent.allow`), and a
whitelist of settings the shell owns (`SHELL_SETTINGS` in
[`queries.rs`](../core/crates/ink-ffi/src/queries.rs), each with the values it takes):
`onboarding.done`, `dictation.polish` and `meetings.llm` (only ever set to off: they turn on
through `consent.allow`), `dictation.key`, `dictation.edit_key`, `dictation.enabled`,
`meetings.detect`, `meetings.headset_mic`, `llm.local_only`, `retention.days`, `import.key_note`,
and the appearance settings: `appearance.mode` (`light`, `dark` or `system`),
`appearance.dots.light` and `appearance.dots.dark` (a preset from
[`design/tokens.json`](../design/tokens.json)), `appearance.you.light`, `appearance.them.light`,
`appearance.you.dark` and `appearance.them.dark` (`preset`, or a `#rrggbb` colour in lowercase),
`appearance.edge_glow` (`on` or `off`) and `appearance.motion` (`system` or `still`). The core
does nothing with the appearance settings itself. A setting never set answers `setting.value`
without a value, and the shell reads it as its default (for appearance, `APPEARANCE_DEFAULTS`:
`system`, `indigo`, `preset`, `on` and `system`).

- They run on their own core thread, `ink-queries`, in order among themselves. The command thread
  can be held for minutes by a model download; a note or a permission card never waits for it.
- A permission check never prompts. System audio has no quiet query: the platform plays a muted
  tone into its own tap and listens for it, which takes about a second, and only once the app has
  asked for System Audio (the core remembers that in the store), because before that the probe
  itself would make macOS prompt. Shells check when the user comes back from System Settings and
  when a screen showing permissions appears, never on a timer.
- A mode names apps by identity (on macOS, bundle ids, or part of one). The shell shows each as the
  app's name and icon; a raw identity is never shown.
- Replies carry the user's words only where the screen asked for them (a commitment's text); a
  note's words are never echoed back, and errors never quote them.

**Dictation** is turned on by the shell (`dictation.enable`, answered by `dictation.ready` or
`dictation.off`) and then lives in the core ([`voice.rs`](../core/crates/ink-ffi/src/voice.rs)):

- **Keys.** The core holds the dictation key and, when one is set, the voice-edit key (two event
  taps under Accessibility). Either may be any key the platform can watch: a right-hand modifier
  (or Fn on the Mac) held on its own, a function key, or modifiers and one key; a chord's hold ends
  when any part of it is let go of. The platform's parser is the one judge
  ([`hotkey.rs`](../core/crates/ink-ffi/src/hotkey.rs)): `hotkey.check` asks it about a shortcut
  the user recorded and answers with its one spelling or why not, and the settings store a key on
  the same rule. The shell only stores the choice (`dictation.key`, `dictation.edit_key`); a
  change rebinds at once. Without Accessibility the answer is `dictation.off` with
  `needs_accessibility`, never a prompt.
- **The mic.** It opens at the first press, not at launch, and stays open so each take keeps the
  300 ms said before its press; after 1 minute without a take it is let go of (an open input keeps
  the Mac awake and the microphone indicator on). The first take after that starts when the device
  does. The ink's bands follow the voice while a take is open.
- **A take.** Live words go to the router's live-partials engine through a live gain stage and
  reach the Drop as `dictation.partial` (never saved; none after `dictation.stopped`). A take's
  start warms the dictation engine after 30 s without a decode (half a second of silence, the
  answer dropped unread; the take's own decode cancels it). A push-to-talk hold past 180 s is taken
  as a lost release: stopped and processed, never discarded.
- **Voice edit.** Select text, hold the edit key, say what to change: the selection (read once the
  hold is confirmed) and the instruction go to the registered language model, and its rewrite
  replaces the selection. Edits are push to talk, one take at a time with dictation, and are not
  saved to the Library.

The Library and a record read through four more, on the same thread: `records.list`,
`records.search`, `record.open` and `library.stats`. Each answer echoes the command's id as `ref`,
and a failure is `command.failed` with that id, so a screen can say "could not load" rather than
show an empty library.

- **Order.** Records list newest first, by start time and then id, with a keyset cursor for the
  next page. The shell never re-sorts by anything else.
- **Summaries are stored as markdown** (a headline, then sections). The shells render them; raw
  markdown never reaches the screen. A meeting's title is its summary's headline unless the
  meeting had one.
- **Audio.** `record.open` lists a meeting's chunk files, each placed on the record's timeline:
  the meeting worker writes the host time of the timeline's zero beside the chunks
  (`timeline.json`), and a chunk's place is its first frame's host time minus that. A record
  without the file (older ones) is placed from its earliest chunk and says so. A chunk whose format
  or host time only recovery could guess is left out and counted, never placed on a guess. The
  audio directory must stay inside the library (no `..`, no absolute path, links resolved). The
  shell plays the chunks from disk a few seconds at a time; a meeting is never read into memory
  whole.
- **Counts.** `library.stats` reads every transcript in its window to count words, so the core
  bounds the window at 31 days and refuses an older moment. (The store has no aggregate query, and
  adding one would widen the store's trait for one screen line.)
- **Words.** These answers carry the library's words (transcripts, notes, summaries, search
  snippets). As with every event that does, they never reach a log.

## Meetings

A meeting records the mic and the far end from this machine (`meeting.start`), or replays two WAV
files through the same path (`replay_meeting`, architecture rule 7). Commands for meetings run on
their own thread, `ink-meetings`, so a model download on the command thread never delays "Record
this call"; questions about a live meeting (`meeting.ask`) run on `ink-ask`.

- **Consent.** Detection only offers. An app that has held the microphone for 3 s is offered
  (`meeting.detected`, the shell's Drop), one at a time; "not this one" lasts until that app
  releases the microphone. Nothing records until the user says so. A meeting recorded for an app
  ends 15 s after the app lets go of the microphone; one started with "Record now" ends when it is
  stopped. The rules are a pure state machine (`ink-ffi/src/detection.rs`); the thread wakes only
  while something is pending. The platform's detector polls the audio server once a second while
  detection is on (`meetings.detect`).
- **Capture.** On the Mac: the routed mic's own IOProc (the built-in mic with Bluetooth output,
  unless `meetings.headset_mic`) and a process tap of the meeting's app, else of everything this
  Mac plays except Inkwell. On Windows: the routed mic (WASAPI), and for the far end process
  loopback of Zoom and the browsers (the app alone) or device loopback of the output any other app
  plays to (everything that device plays, said as such), else of the default output. Device
  loopback moves with the call: the pump asks every 2 s whether its output went or the app plays
  elsewhere, and hands the side's ring to the new source. A side left with no source (it ended by
  itself, or could not be opened again) must deliver from then on, so the watchdog says its
  silence. `meeting.started` names the title (the calendar's event on now, from the shell), the
  app and the mic.
- **What it runs on.** The VAD (Silero), loaded at the start; the far end's diarizer (Nemotron),
  loaded only for the final pass and let go of after it; and the language model the shell
  registered, for the summary, commitments and Ask, sized to its context (`context_tokens`: the
  on-device model holds 4,096 tokens, so a long meeting's summary is written in windows cut by
  size and combined in groups). Foundation Models generates structured answers to the request's
  JSON Schema. Any of them missing is said, never guessed around.
- **The far end's bands** are published beside the mic's (`ink_far_bands_read`), so each drop in
  the ink pulses with its own side.
- **Crash recovery.** A marker (`live.json`) sits beside a meeting's chunks from its start until
  its final pass has run. After a crash, `meetings.recover` (which the shell sends once its own
  engines are registered) repairs the chunks (torn tails trimmed, torn headers rebuilt), ends the
  record where its audio ends, and runs the final pass over what is on disk. Chunks are written
  block by block with no buffer in between, so a killed process loses what was still in the
  capture ring: measured at 0.03 s by a test that kills a real child process mid-meeting; the
  limit is one chunk (10 s).
- **Retention.** `retention.days` (forever by default, or 7, 30, 90, 365 days) deletes whole
  meetings and dictations older than that (never an import: the user's own file, perhaps its only
  copy), at launch, after each meeting's final pass (a recovered one's too) and when it changes, on
  its own thread (a sweep never keeps a meeting "running"): the store's delete overwrites the text
  in the database and checkpoints the write-ahead log with TRUNCATE, then the audio directory is
  removed (a record without an end is never swept).
- **Summaries keep their citations.** Each decision and action is saved with the span of the line
  it cites, so a record shows it; a promise names who it is owed to; a later meeting in which the
  user says an open promise is already done marks it "looks done" for the user to confirm. A
  meeting with no line of at least three words (nothing said, or a noise heard as "Oh.") asks no
  model: it gets no summary, title or commitments.

## Testing

- `cargo test --workspace` in `core/`. CI runs it with fmt, `clippy -D warnings` and `cargo deny` on
  macOS 26 and Windows Server 2025 (`.github/workflows/core.yml`).
- `windows/`: `dotnet test Inkwell.slnx`, run in `windows/` (its `global.json` pins the SDK), after
  `cargo build -p ink-ffi --lib` (the tests load the core's DLL). CI (`.github/workflows/win.yml`, Windows Server 2025) also checks that the
  generated C# is current and runs the NuGet licence check, `windows/scripts/nuget-licences.ps1`.
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
| I2 | Licences clean | `cargo deny`; the Swift audit (`mac.yml`); the NuGet check (`win.yml`) |
| I3 | No private data in the repo | a local pre-push check |
| I4 | Realtime callbacks allocation-free | a thread-scoped guard around every audio callback |
| I5 | No transcripts in logs | a privacy lint test |
| I6 | The legacy app is untouched until 1.0 | no commits to `src/` or `src-tauri/` |
| I7 | Shell budget as shipped | idle: 0 frames and < 0.1 % CPU over 2 min; live at 60 fps: ≤ 6 % p50 and ≤ 10 % p95 of one core; memory ≤ 150 MB plus the model |
