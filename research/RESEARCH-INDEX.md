# Inkwell Post-MVP Research Index

All research completed 2026-03-26. Each topic has a detailed implementation plan in `raw/`.

## Feature Research Files

| # | Feature | File | Key Tech |
|---|---------|------|----------|
| 01 | AI Polish (BYOK LLM) | `raw/01-ai-polish.md` | reqwest + reqwest-eventsource, keyring crate, OpenAICompatible + Anthropic trait |
| 02 | Chainable AI Transforms | `raw/02-chainable-transforms.md` | Sequential pipeline, Tauri Channels for streaming, handlebars templates |
| 03 | Per-App Style Overrides | `raw/03-per-app-styles.md` | Win32 GetForegroundWindow chain, windows crate, process name matching |
| 04 | Snippets Engine | `raw/04-snippets.md` | aho-corasick multi-pattern matching, post-STT pipeline step |
| 05 | Voice Commands | `raw/05-voice-commands.md` | Post-STT wake word detection, whitelist-only execution, tiered safety |
| 06 | File Transcription | `raw/06-file-transcription.md` | symphonia (pure Rust decoder), VAD-based chunking, two-pass for large files |
| 07 | Export (TXT/SRT/JSON/CSV) | `raw/07-export.md` | csv crate, tauri-plugin-dialog, sentence-proportional SRT timestamps |
| 08 | Real-time Streaming | `raw/08-streaming.md` | OnlineRecognizer (Streaming Zipformer), 200ms chunks, partial+final events |
| 09 | Landing Page | `raw/09-landing-page.md` | Astro 5 + React islands, R3F ink shader, GSAP ScrollTrigger |

## Implementation Priority (recommended)

### Tier 1: High impact, builds on existing code
1. **Export** (07) - quick win, users need to get data out
2. **AI Polish** (01) - BYOK is the monetization path
3. **Snippets** (04) - fits naturally in the existing pipeline

### Tier 2: Significant features
4. **Per-App Styles** (03) - differentiator, simple Win32 calls
5. **Chainable Transforms** (02) - builds on AI Polish
6. **File Transcription** (06) - new use case, symphonia handles most formats

### Tier 3: Advanced / separate projects
7. **Real-time Streaming** (08) - needs new model + pipeline rework
8. **Voice Commands** (05) - complex UX, security considerations
9. **Landing Page** (09) - separate project, ship when app is ready

## Common Dependencies Across Features

| Crate | Used By |
|-------|---------|
| `reqwest` + `reqwest-eventsource` | AI Polish, Chainable Transforms |
| `keyring` | AI Polish (API key storage) |
| `handlebars` | Chainable Transforms (prompt templates) |
| `aho-corasick` | Snippets, Voice Commands |
| `windows` (Win32) | Per-App Styles (already a dep via enigo) |
| `symphonia` | File Transcription |
| `csv` | Export |
| `tauri-plugin-dialog` | Export, File Transcription |
| `chrono` | AI Polish (usage tracking), Export |
