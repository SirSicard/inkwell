# Contributing to Inkwell

Inkwell is free, MIT licensed and maintained by one person. Contributions are welcome, and so is a good bug report.

## Reporting bugs

- Search [existing issues](https://github.com/SirSicard/inkwell/issues) first
- Include the OS and its version, the Inkwell version, and steps to reproduce
- For audio problems, say which microphone. Bluetooth headsets behave differently from built-in mics.
- Screenshots and short screen recordings help more than prose

## Suggesting features

Open an issue describing the problem, not just the solution. [docs/ROADMAP.md](docs/ROADMAP.md) shows what is planned, and some things are deliberately out of scope (any paid tier, for one): saying so early saves both of us time.

## Pull requests

1. **Claim the issue first.** Comment on it so nobody duplicates work.
2. Fork, branch from `main`.
3. Keep commits small and focused. One change per commit, imperative mood ("Add X", not "Added X").
4. Run the checks for what you changed (below) and exercise a UI change by hand.
5. Open a PR that says what changed and why.

Two things that will get a PR sent back regardless of how good the code is:

- **Weakening a test to make it pass.** The 57 tests in `core/crates/ink-pipeline/tests/pipeline_tests.rs`, ported from 0.2, are the regression floor. If your change makes one genuinely obsolete, delete that test and say so in the PR.
- **Anything that sends user data anywhere new.** Audio stays local, period. Any new outbound call needs to be off by default, explained to the user, and argued for in the PR.

## Dev setup

Prerequisites: the Rust toolchain (rustup.rs; `core/rust-toolchain.toml` pins the version). For the
Mac app, a Mac with Apple silicon on macOS 26 or later, with Xcode. For the Windows app, Windows 11
24H2 or later, the Visual Studio Build Tools and the .NET SDK that `windows/global.json` pins.

```bash
git clone https://github.com/SirSicard/inkwell.git
cd inkwell

# The core: no models, GPU or audio devices needed (the mock engine and replay fixtures stand in)
(cd core && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace)

# The Mac app: builds mac/build/Inkwell.app (ad-hoc signed), then the Swift tests
mac/scripts/build-mac.sh
swift test --package-path mac

# The Windows app, in windows/ after `cargo build -p ink-ffi --lib` in core/
dotnet test Inkwell.slnx
```

Real speech models are downloaded by the app when first needed, never committed. Tests that need them
are `#[ignore]` and read `$INK_BENCH_DIR`.

The OS permissions (Microphone, System Audio, Accessibility) are granted to a signed app: an ad-hoc
build is a new app to macOS each time it is rebuilt, so its grants reset. Capture, paste and hotkey
behaviour is checked by hand from the checklists in `mac/` and `windows/`.

## Project structure

```
core/                   Rust workspace: capture, audio, echo, store, engines, language models,
                        pipelines, the platform code for each OS, and the C ABI (ink-ffi)
mac/                    The Mac app: SwiftUI and AppKit, the Apple engines, the Metal ink
windows/                The Windows app: WinUI 3 (C#), the Direct3D 11 ink
schema/                 The event schema the Swift and C# types are generated from
shaders/                The ink shader, one WGSL source for both apps
fixtures/               Synthetic and public-licensed audio for tests
homepage/               The website
docs/                   Architecture, releasing, model weights, roadmap
```

Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) before a structural change. It states the rules the code keeps (the core owns time, disk is the seam, nothing draws while idle, and the rest), so you do not have to guess whether the pattern you are copying is the one to keep.

## Code style

- Rust: `cargo fmt` and `cargo clippy -D warnings` before committing. Match the surrounding style over any personal preference.
- Swift and C#: consistency with neighbouring files wins.
- Comments explain constraints the code cannot show. Do not narrate what the next line does.
- New dependencies need a sentence of justification in the PR, and a licence on the allowlist: MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC or Zlib. CI checks Rust, Swift and NuGet licences.

## Maintainer notes

Not needed for contributing, kept here so the release process is written down somewhere.

- **Releases** are cut by pushing a `v1.X.Y` tag, as [docs/RELEASING.md](docs/RELEASING.md) describes. The 0.2 app's releases come from the `legacy/0.2` branch.
- **The Mac updater's signing key** is Sparkle's EdDSA key, whose public half is in `mac/Info.plist`. Losing the private half permanently breaks updates for every installed copy. Keep a backup outside GitHub Actions secrets.
- **Every release** updates `CHANGELOG.md` (Keep a Changelog format). The version comes from the tag.
- **Repo settings:** description "Local-first speech to text for desktop. Free and open source." Topics: `speech-to-text`, `stt`, `dictation`, `tauri`, `rust`, `desktop-app`, `privacy`, `local-first`, `voice`, `transcription`. Discussions on, private vulnerability reporting on.
- **Do not** add a CLA, stale bots, or fifteen labels before there are fifteen issues.

## License

By contributing you agree that your contribution is licensed under the [MIT License](LICENSE).
