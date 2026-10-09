# Contributing to Inkwell

Inkwell is free, MIT licensed and maintained by one person. Contributions are welcome, and so is a good bug report.

## Reporting bugs

- Search [existing issues](https://github.com/SirSicard/inkwell/issues) first
- Include OS and version (macOS is the primary platform, so say which chip), Inkwell version, the model you were using, and steps to reproduce
- For audio problems, say which microphone. Bluetooth headsets behave differently from built-in mics.
- Screenshots and short screen recordings help more than prose

## Suggesting features

Open an issue describing the problem, not just the solution. Check [docs/ROADMAP.md](docs/ROADMAP.md) first: it may already be planned, or deliberately left out, and saying so early saves both of us time.

## Pull requests

1. **Claim the issue first.** Comment on it so nobody duplicates work.
2. Fork, branch from `main`.
3. Keep commits small and focused. One change per commit, imperative mood ("Add X", not "Added X").
4. Run the checks for what you changed (below) and exercise a UI change by hand.
5. Open a PR that says what changed and why.

Two things that will get a PR sent back regardless of how good the code is:

- **Weakening a test to make it pass.** The 57 tests in `core/crates/ink-pipeline/tests/pipeline_tests.rs` are the regression floor. If your change makes one genuinely obsolete, delete that test and say so in the PR.
- **Anything that sends user data anywhere new.** Audio stays local, period. Any new outbound call needs to be off by default, explained to the user, and argued for in the PR.

## Dev setup

Prerequisites: the Rust toolchain (rustup.rs; `core/rust-toolchain.toml` pins the version); for
the Mac app, Xcode 26 (Swift 6.2) on macOS 26; for the Windows app, the .NET SDK that
`windows/global.json` pins.

```bash
git clone https://github.com/SirSicard/inkwell.git
cd inkwell

# The core
(cd core && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace)

# The Mac app: the core as an xcframework, then the Swift tests; build-mac.sh makes mac/build/Inkwell.app
mac/scripts/build-core.sh
swift test --package-path mac
mac/scripts/build-mac.sh

# The Windows app: the core's DLL, then the solution (run dotnet in windows/, where global.json is)
(cd core && cargo build -p ink-ffi --lib)
(cd windows && dotnet build Inkwell.slnx && dotnet test Inkwell.slnx --no-build)
```

Model weights are downloaded at runtime, never committed ([docs/MODEL-WEIGHTS.md](docs/MODEL-WEIGHTS.md)).

On macOS you also need to grant Microphone and Accessibility permission to the dev build, otherwise recording or pasting will silently do nothing.

## Project structure

```
core/                   The Rust core: capture, VAD, echo cancellation, engines, pipeline, store, C ABI (ink-ffi)
mac/                    The Mac app (Swift, SwiftUI and AppKit), built with SwiftPM
windows/                The Windows app (C#, WinUI 3)
schema/                 The event schema the Swift and C# types are generated from
shaders/                The ink shader (WGSL)
fixtures/               Public-licensed audio for the replay tests
homepage/               The website (Astro), its own project
docs/                   Architecture, roadmap, releasing, research archive
```

The 0.2 Tauri app lives on the `legacy/0.2` branch.

Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) before a structural change. It states the rules the code keeps, so you do not have to guess whether the pattern you are copying is the one to keep.

## Code style

- Rust: `cargo fmt` before committing. Match the surrounding style over any personal preference.
- Comments explain constraints the code cannot show. Do not narrate what the next line does.
- New dependencies need a sentence of justification in the PR, and a licence on the allowed list (MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC or Zlib).

## Maintainer notes

Not needed for contributing, kept here so the release process is written down somewhere.

- **Releases** are cut by pushing a `v1.X.Y` tag: `mac-release.yml` and `win-release.yml` draft one release for both apps. [docs/RELEASING.md](docs/RELEASING.md) has the whole chain, the update key's care included, and the 0.2 chain, which runs from `legacy/0.2`.
- **Every release** updates `CHANGELOG.md` (Keep a Changelog format).
- **Repo settings:** description "Local-first speech to text for desktop. Free and open source." Topics: `speech-to-text`, `stt`, `dictation`, `tauri`, `rust`, `desktop-app`, `privacy`, `local-first`, `voice`, `transcription`. Discussions on, private vulnerability reporting on.
- **Do not** add a CLA, stale bots, or fifteen labels before there are fifteen issues.

## License

By contributing you agree that your contribution is licensed under the [MIT License](LICENSE).
