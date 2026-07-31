<p align="center">
  <h1 align="center">INKWELL</h1>
  <p align="center">Local-first speech to text for your desktop. Free, open source, no account.</p>
</p>

<p align="center">
  <a href="https://github.com/SirSicard/inkwell/releases/latest"><img src="https://img.shields.io/github/v/release/SirSicard/inkwell?style=flat-square" alt="Release"></a>
  <a href="https://github.com/SirSicard/inkwell/actions"><img src="https://img.shields.io/github/actions/workflow/status/SirSicard/inkwell/build.yml?style=flat-square" alt="Build"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/SirSicard/inkwell?style=flat-square" alt="License"></a>
</p>

**Your voice, your words, your machine.**

Hold a hotkey, speak, release. The text lands in whatever app you were typing in. Speech recognition runs on your own CPU, so your audio never leaves the machine.

Inkwell is free and stays free. MIT licensed, no paid tier, no license keys, no accounts, no telemetry.

<p align="center">
  <img src="docs/media/inkwell-dashboard.png" alt="Inkwell showing transcript history, with the sidebar and the ink panel" width="900">
</p>

## What it does

- **Dictation anywhere.** Global hotkey, push to talk or toggle. Transcribes, then pastes into the focused app.
- **Local speech recognition.** 13 models via [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx): Parakeet V3 (default), Parakeet V2, Whisper (8 variants), Moonshine Tiny/Base, SenseVoice. Download and switch in the app.
- **Style formatting.** Formal, Casual, Relaxed. Controls capitalization and punctuation without touching a model.
- **Custom dictionary.** Fix the words your model keeps getting wrong. Case insensitive, word boundary matching.
- **Snippets.** Trigger phrases expand to full text. Variables: `{date}`, `{time}`, `{clipboard}`.
- **Voice commands.** Wake prefix plus action, for example "inkwell, scratch that" or "inkwell, formal mode".
- **Per-app styles.** Different style per application. Windows only today, macOS support is on the roadmap.
- **File transcription.** Drag in audio or video. MP3, WAV, FLAC, OGG, M4A, MP4, MKV and more.
- **History and export.** SQLite-backed transcript history, searchable and editable. Export TXT, SRT, JSON, CSV.
- **Tray and overlay.** Lives in the tray. A small always-on-top overlay shows recording state. The hotkey works with the window hidden.
- **AI polish (optional, off by default).** Bring your own API key to clean up grammar, filler words and false starts. See below.

## Privacy

- Audio is captured, resampled and transcribed locally. It is never uploaded.
- Transcripts live in a local SQLite file in your app data directory. Nothing syncs.
- There is no telemetry, no analytics, no crash reporting, no account, no server owned by this project that your text passes through.
- **AI polish is the only feature that talks to the internet, and only if you turn it on.** You supply your own API key for OpenAI, Groq, Anthropic, OpenRouter or a custom OpenAI-compatible endpoint. The key is stored in the OS keyring. When polish is on, the transcribed **text** (never the audio) is sent directly from your machine to the provider you chose. Turn it off and Inkwell makes no network calls except model downloads and update checks.
- Earlier builds shipped a free proxy tier that routed polish requests through a server the maintainer paid for. That is gone. BYOK is the only path.

## Install

Builds are on the [Releases page](https://github.com/SirSicard/inkwell/releases). Nothing is code signed yet, so both platforms will warn you on first launch. Signing (Apple Developer ID plus notarization, and a Windows certificate) is planned but not done.

### macOS (Apple Silicon, primary platform)

1. Download the `.dmg`, drag Inkwell to Applications.
2. Gatekeeper will refuse to open it because the app is unsigned. Either right click the app and choose Open, or clear the quarantine attribute:
   ```bash
   xattr -cr /Applications/Inkwell.app
   ```
3. Grant **Microphone** access when prompted (System Settings > Privacy & Security > Microphone).
4. Grant **Accessibility** access (System Settings > Privacy & Security > Accessibility). Inkwell types the result into the focused app with a synthetic paste, which macOS blocks until this is granted. Without it, transcription works but nothing appears.

Known macOS limitations: synthetic paste is blocked by Secure Input, so dictation into password fields and some terminals will silently do nothing. Per-app style overrides do not work yet.

### Windows (secondary, built in CI)

Download the NSIS installer (recommended) or the MSI. SmartScreen will show a warning on an unsigned installer: click "More info" then "Run anyway".

### Linux

Best effort. CI produces `.AppImage`, `.deb` and `.rpm`. Not regularly tested.

## Quick start

1. Launch Inkwell and finish the short onboarding (mic picker, model download, hotkey test).
2. Pick a model. Parakeet V3 (670 MB) is the recommended default. Models are downloaded on first use, they are not shipped inside the installer.
3. Set your record hotkey in Settings > General. On macOS pick a combination that does not collide with Spotlight or input source switching.
4. Hold the hotkey, speak, release. The text is pasted where your cursor is.

## Models

| Model | Size | Languages | Notes |
|---|---|---|---|
| **Parakeet V3** | 670 MB | 25 European | Default. Best accuracy/speed balance |
| Parakeet V2 | 670 MB | English | English-specialized variant |
| Whisper Turbo | 800 MB | 99 | Good all rounder |
| Whisper Large V3 | 1.5 GB | 99 | Best multilingual accuracy, slow |
| Whisper Medium / Small / Base / Tiny | 1.0 GB / 375 MB / 135 MB / 98 MB | 99 | Size and speed tradeoffs |
| Whisper Distil Medium / Small (EN) | 460 MB / 180 MB | English | Distilled for speed |
| Moonshine Base | 288 MB | English | Fast, moderate size |
| Moonshine Tiny | 70 MB | English | Smallest, lowest accuracy |
| SenseVoice | 160 MB | zh, en, ja, ko, yue | Very fast |

All models run locally on CPU. No internet is needed once a model is downloaded. The catalog is deliberately going to shrink, see [TODO.md](TODO.md).

## Not built (so you do not have to ask)

- **Streaming / live partial text while you speak.** Not implemented. Transcription starts when you release the hotkey.
- **Speaker diarization, meeting mode, calendar integration.** Not planned.
- **GPU acceleration.** CPU inference only.
- **Voice agent mode.** Removed in this rehaul. It targeted a gateway that no longer exists.
- **Silence trimming (Silero VAD)** downloads its model (`silero_vad.onnx`) on first run. Until that finishes, dictation works but silence is not trimmed, and the app says so rather than failing quietly.

## Voice editing

Select text anywhere, hold the edit hotkey (Cmd+Shift+E on macOS, Ctrl+Shift+E elsewhere), say what to change, and the rewrite replaces the selection. "Make this shorter", "fix the grammar", "turn this into bullet points".

This needs an API key, because rewriting text to order is a language-model job and Inkwell has no model of its own to do it with. Dictation itself stays entirely local and needs no key. Clear the hotkey in General to turn the feature off and free the shortcut.

## Support the project

Inkwell is free and will not be paywalled. If it saves you time, you can leave a tip:

> **https://buymeacoffee.com/mattiasherzig**

Suggested: EUR 10. Entirely voluntary, nothing in the app is gated behind it.

Non-financial help is worth more: file a bug, report what breaks on your hardware, or send a PR.

## Build from source

```bash
# Prerequisites: Rust toolchain (rustup.rs), Node.js 20+
# Platform deps: https://v2.tauri.app/start/prerequisites/

git clone https://github.com/SirSicard/inkwell.git
cd inkwell
npm install
cargo tauri dev
```

Rust tests: `cargo test` in `src-tauri`. See [CONTRIBUTING.md](CONTRIBUTING.md) for the codebase layout and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for where the code is heading.

## Stack

Rust + [Tauri v2](https://tauri.app), [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) for inference (ONNX Runtime, CPU), [Silero VAD](https://github.com/snakers4/silero-vad), cpal for capture, rubato for resampling, SQLite for history. Frontend is React 19, TypeScript, Tailwind v4, Framer Motion, and a WebGL ink shader.

## Requirements

- macOS on Apple Silicon, Windows 10/11, or a recent Linux desktop
- 2 GB free RAM with Parakeet V3, less with the small models
- Disk: about 50 MB for the app plus the model you choose (70 MB to 1.5 GB)
- Any microphone

## Contributing

Bug reports and PRs are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) first. Security issues go through [SECURITY.md](SECURITY.md), not the public issue tracker.

## License

MIT. See [LICENSE](LICENSE).

## Credits

Built by [Mattias Herzig](https://mattiasherzig.com). Originally based on [Handy](https://github.com/cjpais/Handy) by CJ Pais. Powered by [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx), [Tauri](https://tauri.app) and [Silero VAD](https://github.com/snakers4/silero-vad).
