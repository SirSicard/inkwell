# Contributing to Inkwell

Inkwell is free, MIT licensed and maintained by one person. Contributions are welcome, and so is a good bug report.

## Reporting bugs

- Search [existing issues](https://github.com/SirSicard/inkwell/issues) first
- Include OS and version (macOS is the primary platform, so say which chip), Inkwell version, the model you were using, and steps to reproduce
- For audio problems, say which microphone. Bluetooth headsets behave differently from built-in mics.
- Screenshots and short screen recordings help more than prose

## Suggesting features

Open an issue describing the problem, not just the solution. Check [PRD.md](PRD.md) and [TODO.md](TODO.md) first: some things are deliberately out of scope (meeting mode, diarization, agent mode, any paid tier), and saying so early saves both of us time.

## Pull requests

1. **Claim the issue first.** Comment on it so nobody duplicates work.
2. Fork, branch from `main`.
3. Keep commits small and focused. One change per commit, imperative mood ("Add X", not "Added X").
4. Run `cargo test` in `src-tauri` and exercise the UI change by hand.
5. Open a PR that says what changed and why.

Two things that will get a PR sent back regardless of how good the code is:

- **Weakening a test to make it pass.** The 60 tests in `src-tauri/tests/pipeline_tests.rs` are the regression floor. If your change makes one genuinely obsolete, delete that test and say so in the PR.
- **Anything that sends user data anywhere new.** Audio stays local, period. Any new outbound call needs to be off by default, explained to the user, and argued for in the PR.

## Dev setup

```bash
# Prerequisites
# - Rust toolchain (rustup.rs)
# - Node.js 20+
# - Platform deps: https://v2.tauri.app/start/prerequisites/

git clone https://github.com/SirSicard/inkwell.git
cd inkwell
npm install
cargo tauri dev
```

On first run the app asks you to download a model. Parakeet V3 is about 670 MB. Moonshine Tiny (70 MB) is enough for development.

On macOS you also need to grant Microphone and Accessibility permission to the dev build, otherwise recording or pasting will silently do nothing.

## Project structure

```
src/                    React frontend (TypeScript, Tailwind v4, Framer Motion)
  tabs/                 Settings and feature tabs
  components/           UI kit and the InkCanvas WebGL shader
src-tauri/src/          Rust backend
  pipeline.rs           Dictation pipeline (being extracted into a service)
  engine.rs             sherpa-onnx STT engine
  vad.rs                Silero VAD
  recording.rs          Capture and resampling
  filetranscribe.rs     File transcription
  style.rs dictionary.rs snippets.rs voicecommand.rs   Pure text transforms, tested
  llm.rs polish.rs      BYOK AI polish providers
  commands.rs           Tauri commands (being split by domain)
src-tauri/tests/        Pipeline tests
docs/                   Architecture, rehaul analysis, research archive
```

Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) before a structural change. It states where the code is going and which parts of the current code already break those rules, so you do not have to guess whether the pattern you are copying is the one to keep.

## Code style

- Rust: `cargo fmt` before committing. Match the surrounding style over any personal preference.
- TypeScript: no strict linter beyond ESLint. Consistency with neighboring files wins.
- Comments explain constraints the code cannot show. Do not narrate what the next line does.
- New dependencies need a sentence of justification in the PR. The frontend already carries dead ones that are being removed.

## Maintainer notes

Not needed for contributing, kept here so the release process is written down somewhere.

- **Releases** are cut by pushing a `v*` tag. `.github/workflows/build.yml` builds macOS (ARM and Intel), Windows (NSIS and MSI) and Linux (AppImage, deb, rpm), then opens a draft release with updater JSON.
- **The updater signing key** is a minisign key whose public half is pinned in `tauri.conf.json`. Losing the private half permanently breaks updates for every installed copy. Keep a backup outside GitHub Actions secrets.
- **Every release** updates `CHANGELOG.md` (Keep a Changelog format) and the version in `tauri.conf.json`.
- **Repo settings:** description "Local-first speech to text for desktop. Free and open source." Topics: `speech-to-text`, `stt`, `dictation`, `tauri`, `rust`, `desktop-app`, `privacy`, `local-first`, `voice`, `transcription`. Discussions on, private vulnerability reporting on.
- **Do not** add a CLA, stale bots, or fifteen labels before there are fifteen issues.

## License

By contributing you agree that your contribution is licensed under the [MIT License](LICENSE).
