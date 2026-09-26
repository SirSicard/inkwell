# Inkwell: agent rules

Inkwell is a public MIT project: a local voice app for dictation and meeting notes. `main` holds the
Inkwell 1.0 rebuild in `core/` (Rust) alongside the 0.2 Tauri app (`src/`, `src-tauri/`), which is
frozen on the `legacy/0.2` branch. Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) before a
structural change.

## Architecture rules (the short form)

1. The core owns time: capture, ring, VAD, echo cancellation, engines, the hotkey state machine and
   insertion are Rust. Shells render state and send commands.
2. Apple accelerators (FluidAudio, Foundation Models) stay in Swift and register into the core as
   engines over the C ABI. The core never wraps CoreML.
3. Disk is the seam: raw PCM chunks on disk, seconds in RAM, never a session.
4. Partials are ephemeral and never stored; finals persist. The offline pass supersedes the live one
   in one transaction and refuses an empty result or one under half the previous words.
5. Me versus them is stream identity. Only the far end is diarized; labels need at least two
   clusters holding at least 2 % of the speech each.
6. Local-only mode refuses any non-loopback model endpoint, in code.
7. One replay harness (`FileReplaySource`) on both OSes.
8. One event schema in `schema/`; Swift and C# types are generated from it.
9. Draw nothing when idle. No polling timers.
10. Engines per job and 11. a gain stage before every engine: see the architecture doc.

Every trait in `core/crates/ink-core` names the thread each method may run on (realtime, pump,
worker, callback, main). Respect it: nothing on a realtime thread allocates, locks, blocks or logs.

## Licences

- **Code dependencies:** MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC or Zlib only. CI checks
  Rust with `cargo deny` (`core/deny.toml`); Swift and NuGet get their own audits.
- **UniFFI is out** (MPL-2.0). The C ABI is hand-written.
- **Model weights** are downloaded at runtime, never committed. Rules in
  [docs/MODEL-WEIGHTS.md](docs/MODEL-WEIGHTS.md).
- **Never open, quote, paraphrase or adapt code from:** VoiceInk (GPL-3.0, although GitHub's API
  reports `NOASSERTION`), Fluid-oss, Glass/Pickle, cheating-daddy, Pluely, Amurex, voicetypr,
  AudioTee (no licence), Natively, or ScreenPipe after its June 2026 relicence.
- **Permitted references:** AudioCap (BSD-2; keep its notice in the file header and a row in
  [THIRD_PARTY.md](THIRD_PARTY.md)), anarlog (MIT; design only, never verbatim), Handy, Hex,
  Meetily, FluidAudio, WhisperKit, transcribe.cpp (MIT).

## Privacy

This repository is public. Never commit private data: no real meeting audio, no transcripts, no
names or details of real people, companies or clients, no personal contact details, and no paths
from anyone's machine. Examples and fixtures are synthetic or come from public datasets credited in
[fixtures/ATTRIBUTION.md](fixtures/ATTRIBUTION.md). Transcripts and prompts never reach logs or error
messages.

## Working here

- One change per branch and PR. Always `git -C <repo>`.
- Commit author email: `69670970+SirSicard@users.noreply.github.com`. Vercel builds the homepage
  from this repository and refuses commits by other authors.
- Tests first for library code. `cargo test --workspace`, `cargo fmt --all --check` and
  `cargo clippy --workspace --all-targets -- -D warnings` must pass in `core/` before a PR.
- CI has no models, GPU or audio devices: use the mock engine and replay fixtures. Tests that need
  real models are `#[ignore]` and read `$INK_BENCH_DIR`.
- Don't touch `src/` or `src-tauri/` on `main`: the 0.2 app changes only on `legacy/0.2`.
- Downloads of models or datasets need the maintainer's OK first.
