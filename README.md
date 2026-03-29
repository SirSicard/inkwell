# Inkwell

Premium speech-to-text for your desktop. Private by default, beautiful by design.

**Your voice, your words, your machine.**

Everything runs locally. No accounts, no cloud, no data leaving your computer.

## Features

- **Core dictation** - Global hotkey (push-to-talk or toggle), transcribe, paste into any app
- **13 STT models** - Parakeet V3 (recommended), Whisper variants, Moonshine, SenseVoice. Download and switch in-app
- **Style formatting** - Formal, Casual, or Relaxed. Auto-capitalizes, punctuates, or strips it all
- **AI Polish** - Optional LLM cleanup (free tier or bring your own API key). Grammar, filler words, false starts
- **Per-app styles** - Formal in Outlook, casual in Slack, relaxed in Discord. Auto-detects the focused app
- **Snippets** - Trigger phrases that expand to full text. Variables: `{date}`, `{time}`, `{clipboard}`
- **Voice commands** - "Inkwell, scratch that" / "Inkwell, formal mode". Wake prefix + action
- **File transcription** - Drag and drop audio/video files. MP3, WAV, FLAC, M4A, MP4, MKV, and more
- **Custom dictionary** - Fix words the STT gets wrong. Case-insensitive, word-boundary matching
- **Export** - TXT, SRT, JSON, CSV
- **Transcript history** - SQLite-backed, searchable, editable
- **System tray** - Lives in your tray, hotkey works with window hidden
- **Floating overlay** - Minimal recording indicator, always-on-top, non-intrusive

## Screenshots

*Coming soon*

## Install

### Windows

Download the latest installer from [Releases](https://github.com/SirSicard/inkwell/releases).

- **NSIS installer** (7.5 MB) - recommended
- **MSI installer** (10.5 MB) - for enterprise/GPO deployment

> Note: The installer is currently unsigned. Windows SmartScreen may show a warning. Click "More info" then "Run anyway".

### macOS / Linux

Coming soon.

## Quick Start

1. Install and launch Inkwell
2. A lightweight model (Moonshine Tiny, 70 MB) is bundled for instant use
3. Inkwell will offer to download Parakeet V3 (670 MB) for much better accuracy
4. Press **Ctrl+Space** (default) to record, release to transcribe and paste
5. Text appears in whatever app you're typing in

## Models

| Model | Size | Languages | Speed | Notes |
|---|---|---|---|---|
| **Parakeet V3** | 670 MB | 25 European | Fast | Recommended. Best accuracy for English + European languages |
| Parakeet V2 | 670 MB | English | Fast | English-specialized variant |
| Moonshine Base | 288 MB | English | Fast | Good accuracy, moderate size |
| Moonshine Tiny | 70 MB | English | Fastest | Bundled fallback. Lower accuracy |
| Whisper Turbo | 800 MB | 99 languages | Medium | Good all-rounder |
| Whisper Large V3 | 1.5 GB | 99 languages | Slow | Best multilingual accuracy |
| Whisper Small | 375 MB | 99 languages | Fast | Lightweight multilingual |
| SenseVoice | 160 MB | 5 languages | Fastest | Chinese, English, Japanese, Korean, Cantonese |

All models run locally via [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx). No internet required after download.

## Tech Stack

- **Backend:** Rust + [Tauri v2](https://tauri.app)
- **STT engine:** [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) (ONNX Runtime, CPU inference)
- **VAD:** Silero VAD
- **Frontend:** React 19 + TypeScript + Tailwind CSS v4 + Framer Motion
- **Audio:** cpal (capture) + rubato (resampling)
- **Storage:** SQLite (transcripts) + JSON (settings, snippets, dictionary)
- **AI Polish:** OpenAI, Groq, Anthropic, OpenRouter, or custom endpoint (BYOK)

## Requirements

- **OS:** Windows 10/11 (macOS and Linux coming)
- **RAM:** 1 GB minimum (with Moonshine Tiny), 2 GB recommended (with Parakeet V3)
- **Disk:** ~50 MB for app + model size (70 MB to 1.5 GB depending on model)
- **Microphone:** Any input device

## Development

```bash
# Prerequisites: Rust toolchain, Node.js 18+
git clone https://github.com/SirSicard/inkwell.git
cd inkwell
npm install
cargo tauri dev
```

## Privacy

Inkwell processes all speech locally on your machine. Audio never leaves your device.

The optional AI Polish feature sends transcription **text only** (not audio) to an LLM provider when enabled. You can:
- Use the free tier (rate-limited, via Inkwell proxy)
- Bring your own API key (direct to provider, Inkwell never sees your text)
- Disable it entirely (default)

## License

All rights reserved. Free for personal use.

## Credits

Built by [Mattias H.](https://mattiasherzig.com)

Powered by open-source: [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx), [Tauri](https://tauri.app), [Silero VAD](https://github.com/snakers4/silero-vad).
