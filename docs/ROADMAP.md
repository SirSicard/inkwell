# Roadmap: Inkwell 1.0

Inkwell 1.0 rebuilds the app as one local voice tool for dictation and meeting notes, with native
shells over a shared Rust core ([ARCHITECTURE.md](ARCHITECTURE.md)). No dates are promised; the
order follows dependencies. English only in 1.0. Step ids (S1.1 …) are the ones the code and
commit messages refer to.

The 0.2 Tauri app keeps shipping from the `legacy/0.2` branch until 1.0 replaces it. Its own list of
work is [TODO.md](../TODO.md).

## M1: the core

The Rust workspace in `core/`, green on macOS and Windows with no models or audio devices in CI.

- **S1.1** Foundation: every crate and every trait, mocks, CI, licence checks.
- **S1.2a** Capture ring, chunk store and the replay harness.
- **S1.2b** Audio processing: resampler, downmix, the gain stage, VAD.
- **S1.3** Store: SQLite with full-text search and the two-pass supersede.
- **S1.4a–c** Engines: registry, downloader, router, residency; llama.cpp, sherpa-onnx and NeMo-Speech.cpp
  adapters.
- **S1.6** Language models: bring-your-own-key providers, local-only mode, local summaries, commitments.
- **S1.5a–c** Pipelines: dictation, meetings, file import; echo cancellation and a silent-channel watchdog.
- **S1.7** The C ABI and event schema the shells link.

## M2: Inkwell 1.0 for Mac

macOS 26 or later, Apple silicon.

- **S2.1a–b** Capture (process taps), meeting detection, permission checks; the event tap, insertion and focus.
- **S2.2** Apple engines registered into the core: FluidAudio for live text, Foundation Models for polish.
- **S2.3–S2.4** The shell: menu bar item, main window, the ink renderer and the Drop overlay.
- **S2.5–S2.6** Screens: Today, Library, Record, Live, Owed, Settings, onboarding.
- **S2.7–S2.8** Dictation and meetings end to end, then a week of daily use before release.
- **S2.9a–c** Import from 0.2 history; signed, notarized release with Sparkle updates.
- 0.2.11 on the legacy branch points 0.2 users to 1.0.

## M3: Inkwell for Windows

Windows 11 24H2 or later.

- **S3.1** WASAPI capture and meeting detection, hotkeys and insertion.
- **S3.2** Engines on Vulkan with a CPU fallback.
- **S3.3–S3.4** The WinUI 3 shell, screens, and the ink renderer on Direct3D 11.
- **S3.5–S3.6** Dictation and meetings end to end, daily use, then the Store and a direct installer with signed
  builds.
