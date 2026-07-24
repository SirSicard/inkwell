# Changelog

All notable changes to Inkwell will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.1] - 2026-04-01

### Added

- Audio feedback on hotkey press/release: soft chime for dictation, distinct synth pulse for agent mode. Configurable in General settings.
- Mic device selector in onboarding wizard so Bluetooth headsets and non-default inputs can be chosen during first run.
- New app icon: cream ink drop (visible on dark taskbars and docks).
- Open source files: MIT license, CONTRIBUTING.md, CODE_OF_CONDUCT.md, SECURITY.md, CHANGELOG.md, issue/PR templates.
- Handy (CJ Pais) attribution in LICENSE and About tab.

### Fixed

- macOS overlay transparency: white background no longer visible behind recording indicator.
- Homepage download links: repo is now public, downloads no longer return 404.
- Homepage title overflow on wide desktop screens.
- Homepage dropdown menus clipped by card overflow.
- macOS Gatekeeper warning text updated with correct `xattr -cr` instructions.

[0.1.1]: https://github.com/SirSicard/inkwell/compare/v0.1.0...v0.1.1

## [0.1.0] - 2026-03-31

First public release.

### Added

- **Core dictation**: global hotkey (push-to-talk or toggle), record, transcribe, paste into any app
- **13 STT models**: Parakeet V3 (default), Parakeet V2, Moonshine Tiny/Base, Whisper Turbo/Large V3/Medium/Small/Tiny, SenseVoice, Canary Flash. All local via sherpa-onnx
- **In-app model manager**: download, switch, and remove models. Auto-downloads Parakeet V3 on first launch
- **Style formatting**: Formal (proper caps, full punctuation), Casual (caps, light punctuation), Relaxed (lowercase, minimal)
- **AI Polish**: optional LLM post-processing to clean grammar, filler words, false starts. Free tier via Inkwell proxy (4,000 words/week) or bring your own API key (OpenAI, Groq, Anthropic, OpenRouter, custom endpoint)
- **Snippets engine**: trigger phrases that expand to full text with variable interpolation ({date}, {time}, {clipboard})
- **Voice commands**: wake prefix ("inkwell") + 6 built-in commands (scratch that, formal mode, casual mode, relaxed mode, copy that, pause). Custom commands supported
- **File transcription**: drag and drop audio/video files (MP3, WAV, FLAC, OGG, M4A, MP4, MKV, and more). VAD-chunked processing
- **Export**: TXT, SRT, JSON, CSV with timestamp computation
- **Custom dictionary**: case-insensitive word-boundary replacement for words the STT gets wrong
- **Per-app style overrides**: auto-detect focused application, apply different style settings per app (Windows only, macOS/Linux stubs)
- **Transcript history**: SQLite-backed, searchable, editable, with copy and delete
- **System tray**: app lives in tray, hotkey works with window hidden
- **Floating overlay**: minimal always-on-top recording indicator (dot + timer + audio bars), cursor passthrough
- **Onboarding wizard**: 5-step first-run experience (mic permission, hotkey test, model download)
- **Voice Agent mode**: second hotkey (Ctrl+Shift+Space) to send voice commands to OpenClaw gateway
- **Ink shader**: WebGL simplex noise animation in left panel, frequency-reactive to microphone input
- **Glass UI**: custom component kit (GlassCard, GlassToggle, GlassInput, GlassButton, GlassSelect)
- **Progressive disclosure**: Advanced Mode toggle hides power features from casual users
- **Auto-updater skeleton**: Tauri updater plugin with signed artifacts and update toast UI
- **Cross-platform CI**: GitHub Actions builds Windows (.exe, .msi), macOS (ARM + Intel .dmg), Linux (.AppImage, .deb, .rpm)
- **60 pipeline tests**: style, dictionary, snippets, usage tracking, export, voice commands, recording, full integration

### Known Issues

- Installers are unsigned. Windows SmartScreen and macOS Gatekeeper will show warnings
- Ink shader sensitivity is slightly too high (cosmetic, will be dialed back)
- Per-app style overrides only work on Windows (macOS/Linux return no active app)
- Auto-updater endpoint not yet live (needs domain)
- Parakeet model load time is 18-22 seconds on first use

[0.1.0]: https://github.com/SirSicard/inkwell/releases/tag/v0.1.0
