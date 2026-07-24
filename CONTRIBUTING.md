# Contributing to Inkwell

Thanks for wanting to help. Inkwell is a young project and contributions are welcome.

## Reporting Bugs

- Search [existing issues](https://github.com/SirSicard/inkwell/issues) first
- Include your OS, Inkwell version, and steps to reproduce
- Screenshots or screen recordings help a lot

## Suggesting Features

- Open an issue with the "feature request" label
- Describe the problem you're solving, not just the solution you want

## Pull Requests

1. **Claim the issue first.** Comment on it so we don't duplicate work
2. Fork the repo, create a branch from `main`
3. Keep commits small and focused. One change per commit
4. Test your changes (run `cargo test` for Rust, check the UI manually)
5. Open a PR with a clear description of what changed and why

### Dev Setup

```bash
# Prerequisites
# - Rust toolchain (rustup.rs)
# - Node.js 18+
# - Platform deps: see Tauri v2 prerequisites (https://v2.tauri.app/start/prerequisites/)

git clone https://github.com/SirSicard/inkwell.git
cd inkwell
npm install
cargo tauri dev
```

The app will open with Moonshine Tiny (bundled). For better accuracy, it will prompt you to download Parakeet V3 (~670 MB).

### Project Structure

```
src/              # React frontend (TypeScript + Tailwind)
  tabs/           # Tab components (Dashboard, General, Audio, Models, etc.)
  components/ui/  # Glass UI kit
src-tauri/src/    # Rust backend
  pipeline.rs     # Core recording pipeline
  engine.rs       # STT engine (sherpa-onnx)
  agent.rs        # Voice Agent mode (OpenClaw integration)
  llm.rs          # AI Polish providers
  vad.rs          # Silero VAD
  commands.rs     # All Tauri commands
inkwell-worker/   # Cloudflare Worker (free AI Polish proxy)
```

### Code Style

- Rust: standard `rustfmt`. Run `cargo fmt` before committing
- TypeScript: no strict linter yet, just keep it consistent with existing code
- Commits: imperative mood ("Add feature" not "Added feature")

## Not Sure Where to Start?

Look for issues labeled `good first issue` or ask in the issue thread. No question is too basic.

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](LICENSE).
