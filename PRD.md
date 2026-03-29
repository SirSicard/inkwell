# Inkwell - Product Requirements Document
*v0.3 | 2026-03-29 | Mattias H.*
*Build status: Phases 1-3 COMPLETE. Phase 4 partial (error handling + first-run + Windows build done). v0.2/v0.3 features mostly built ahead of schedule.*

---

## 1. Product Vision

Inkwell is a free, premium speech-to-text desktop application that turns your voice into text anywhere on your computer. Private by default, beautiful by design, powerful when you need it.

**One-liner:** "Your voice, your words, your machine."

## 2. Brand & Identity

- **Name:** Inkwell
- **Creator:** Mattias H.
- **Tagline options:** "Your voice, your words." / "Speak freely." / "Voice to ink."
- **Brand feel:** Premium, minimal, trustworthy. Think Raycast meets Linear. Not a dev tool, not a toy.
- **License:** Closed source, free (v1). Future: free core + premium tier.

## 3. Target Users

### Primary: Knowledge Workers (non-technical)
- Writers, marketers, managers, students
- People who type a lot and wish they didn't
- Privacy-conscious but not paranoid (they just want it local)
- Expect polish. Will leave if the UI feels "open source"

### Secondary: Power Users / Developers
- Want snippets, voice commands, custom models, hotkey choreography
- Willing to dig into settings
- Will find the "Advanced Mode" toggle and love it

### Tertiary (future): Teams / Enterprise
- Meeting transcription, shared dictionaries, compliance
- Not v1. Not v2. Maybe v3.

## 4. Core Principles

1. **Beautiful by default.** First impression is everything. If it doesn't feel premium on launch, we've failed.
2. **Works in 10 seconds.** Install, grant mic permission, press hotkey, speak, see text. No onboarding wizard, no account creation, no model download wait screen blocking first use.
3. **Private by default.** All STT is local. No cloud calls unless the user explicitly opts in to AI features.
4. **Progressive disclosure.** Simple by default, powerful on demand. Non-tech users never see anything scary. Power users find everything they need.
5. **Zero cost to user, minimal cost to us.** No managed cloud tier in v1. All processing is local or BYOK.

## 5. UX Model: Progressive Disclosure

### Default Mode (what everyone sees)
- Clean, minimal interface
- Hotkey to record, release to transcribe
- Style selector (Formal / Casual / Relaxed)
- Transcript history
- Basic settings (mic, hotkey, language, model)

### Advanced Mode (toggle in settings: "Enable Advanced Features")
Unlocks:
- AI Polish (BYOK: paste your OpenAI/Anthropic/Groq key)
- Snippets engine
- Voice commands
- Custom dictionary
- Model management (download/switch/remove models)
- Per-app style overrides
- Audio settings (VAD sensitivity, standby mic, paste method)
- Chainable AI transforms
- Export options

### Future: Agent Mode (post-MVP)
- MCP server connections
- Voice-to-agent bridge
- Smart commands (context-aware routing)
- Meeting mode with diarization

## 6. Feature Specification

### 6.1 Core Dictation (MVP - v0.1)

**Recording**
- Global hotkey to toggle recording (configurable, default: Ctrl+Shift+Space)
- Push-to-talk mode (hold hotkey to record, release to stop)
- Toggle mode (press to start, press again to stop)
- Visual indicator: floating overlay showing recording state (minimal, non-intrusive)
- Audio level meter during recording

**Transcription**
- Local inference using bundled/downloaded models
- VAD (Voice Activity Detection) to strip silence
- Paste result into currently focused text field
- Clipboard fallback if direct paste fails
- Show transcription result briefly before paste (toast/overlay)

**Models (ship with / downloadable)**
- Moonshine Tiny int8 V1 layout (31MB) - working fallback. HF: csukuangfj/sherpa-onnx-moonshine-tiny-en-int8
- ~~Moonshine V2 Medium (192MB)~~ - not yet integrated
- ~~Whisper Small (487MB)~~ - not yet integrated
- ~~Whisper Turbo (1.6GB)~~ - not yet integrated
- ~~Whisper Large v3 Q5_0 (1.1GB)~~ - not yet integrated
- **Parakeet V3 (678MB total, int8) - CURRENT DEFAULT.** HF: csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8. Load ~20s, 720MB RAM. Excellent English accuracy. Manual download only (no in-app downloader yet).
- ~~SenseVoice (160MB)~~ - not yet integrated

**Note on model spec:** PRD originally listed Parakeet at 478MB. Actual int8 download is ~678MB (encoder 652MB + decoder 11.8MB + joiner 6.4MB + tokens). Spec was based on fp16 estimate.

**Style Formatting**
- Three modes: Formal / Casual / Relaxed
  - Formal: proper caps, full punctuation, clean sentences
  - Casual: caps, lighter punctuation
  - Relaxed: no caps, minimal punctuation
- Quick toggle from main UI (not buried in settings)

### 6.2 Interface

**Main Window**
- Dashboard: transcript history feed (timestamped, searchable)
- Each transcript: copy button, edit inline, delete
- Compact mode: just the recording indicator, no window

**System Tray**
- Always accessible from tray
- Quick actions: start/stop, change style, open settings
- Status indicator (idle, recording, processing)

**Floating Overlay (during recording)**
- Minimal pill/bubble showing: recording state + duration + audio level
- Configurable position (corner, center, hidden)
- Should feel like a native OS element, not a web popup

**Settings**
- Tabbed: General, Audio, Models, AI (advanced), Tools (advanced), About
- General: hotkey, style, language, startup behavior, overlay position
- Audio: mic selection, VAD sensitivity, standby mic, mute while recording
- Models: download/manage/switch, show size + speed + accuracy rating
- AI (advanced only): BYOK keys, polish prompt, transform chains
- Tools (advanced only): snippets, voice commands, dictionary
- About: version, credits, links

### 6.3 Transcript Management

- Full history of all transcriptions with timestamps
- Search across history
- Copy any past transcript
- Edit transcripts inline
- Delete individual or bulk
- Export history (TXT, JSON, CSV)
- Auto-cleanup option (delete after X days)

### 6.4 Advanced Features (behind toggle)

**AI Polish (BYOK)**
- Bring your own API key (OpenAI, Anthropic, Groq, Cerebras, OpenRouter, custom endpoint)
- Post-processes raw transcription through LLM
- Configurable prompt ("clean up grammar", "make professional", "summarize", custom)
- Chainable transforms: raw → clean grammar → format for email → paste
- Per-app polish profiles (formal in Gmail, casual in Slack)
- Clear indicator when AI polish is active vs raw transcription

**Snippets**
- Trigger phrases that expand to full text
- Example: say "email sign-off" → pastes full signature block
- Manage in settings: trigger phrase + expansion text
- Import/export snippet sets

**Voice Commands**
- Keyword → action mapping
- Built-in: "undo that" (remove last transcription), "clear" (clear buffer)
- Custom: "open notes" → launch app, "open gmail" → open URL
- Configurable in settings

**Custom Dictionary**
- Words the STT consistently gets wrong
- Replacement pairs: "Inkwell" not "ink well", "Mattias" not "Matthias"
- Auto-learn option (suggest corrections based on manual edits)

**Per-App Style Overrides**
- Detect which application is focused
- Apply different style/polish settings per app
- Example: Slack = casual, Outlook = formal, VS Code = raw

### 6.5 File Transcription (v1.1+)
- Drag and drop audio/video files
- Batch transcription
- Speaker diarization
- Export: TXT, SRT, VTT, JSON
- Progress indicator for long files

### 6.6 Meeting Mode (v2+)
- Continuous recording with speaker diarization
- Calendar integration (auto-start for scheduled meetings)
- Live captions overlay
- AI summary + action items (BYOK)
- Fathom/Otter-style experience but local

### 6.7 Agent Mode (v2+)
- MCP server connections (connect to any MCP-compatible tool)
- Voice-to-agent bridge: speak commands, agent executes
- Smart commands: context-aware routing ("email John about the meeting" → opens email client)
- Separate hotkey for agent mode vs dictation mode
- Visual feedback: what the agent is doing

## 7. Technical Constraints (no stack specified yet)

### Must Have
- Cross-platform: Windows (primary), macOS, Linux
- Lightweight: <50MB installer, <200MB RAM idle
- Fast: <2 second transcription for 10 seconds of speech
- Offline: core STT works with zero internet
- Accessible: keyboard-only navigation, screen reader friendly
- Auto-update mechanism
- Single instance (prevent multiple copies running)

### Should Have
- GPU acceleration when available (CUDA, Metal, Vulkan)
- Graceful fallback to CPU-only
- Model hot-swap without restart
- Configurable data storage location
- Portable mode (run from USB, no install)

### Won't Have (v1)
- Account system / login
- Cloud-managed anything
- Telemetry (unless opt-in analytics later)
- Mobile app
- Browser extension

## 8. Competitive Positioning

### What Makes Inkwell Different

| vs Wispr Flow | Local/private, free, no 6-min cap, no cloud dependency |
| vs SuperWhisper | Beautiful UX (not power-user-complex), Windows support, free |
| vs Spokenly/Voibe | Cross-platform (not Mac-only), premium feel |
| vs Handy/Whispering | Premium UX, AI polish, snippets, progressive disclosure |
| vs Walkie | Original product, polished UI, clear roadmap to agent/meeting |
| vs SpeechPulse | Modern UI, not a Dragon replacement, extensible |

### The Gap We Fill
Premium + Private + Cross-platform + Free. Nobody is here.

## 9. Success Metrics (v1 launch)

- Install to first transcription: <30 seconds
- Transcription accuracy: match or exceed Whisper Turbo baseline
- App startup time: <2 seconds
- Memory usage idle: <150MB
- Bundle size: <50MB (without models)
- User can complete full flow without reading docs

## 10. Milestones

### v0.1 - MVP ("It works")
**Status: Phases 1-3 COMPLETE. Phase 4 partially done. Most v0.2/v0.3 features also built.**
- [x] Core dictation (hotkey, record, transcribe, paste)
- [x] Moonshine Tiny bundled fallback + Parakeet V3 default (auto-download)
- [x] Whisper integration with GPU detection (11 models total)
- [x] In-app model downloader + manager UI (download/switch/remove)
- [x] Style formatting (Formal / Casual / Relaxed)
- [x] System tray + floating overlay
- [x] Settings (full: General, Audio, Models, Advanced)
- [x] Transcript history (SQLite, search, edit, copy, delete)
- [x] Advanced mode toggle (progressive disclosure)
- [x] Custom dictionary
- [x] Error handling (4.1) — mic disconnect, model fallback, paste fail
- [x] First-run experience (4.2) — onboarding wizard, model download prompt
- [x] Windows build (4.3) — NSIS + MSI, unsigned (SmartScreen warning)
- [ ] Windows code signing (4.3.4-5) — deferred, Certum ~€30/yr
- [ ] macOS build + notarization (4.4)
- [ ] Linux build (4.5)
- [ ] Auto-update (4.6)
- [ ] Final polish + README (4.7)
- [ ] Git repo — no version control yet

### v0.2 - Polish ("It's good") — MOSTLY SHIPPED IN v0.1
- [x] Transcript history + search
- [x] Advanced mode toggle
- [x] AI Polish (BYOK: OpenAI, Groq, Anthropic, OpenRouter, Custom + free proxy tier)
- [x] Custom dictionary (case-insensitive, word-boundary matching)
- [x] Per-app style overrides (Windows only, macOS stub returns None)
- [ ] Linux build
- [ ] Auto-update

### v0.3 - Power ("It's powerful") — MOSTLY SHIPPED IN v0.1
- [x] Snippets engine (triggers, variable interpolation: {date}, {time}, {clipboard})
- [x] Voice commands (wake prefix "inkwell", 6 defaults, Aho-Corasick matching, tests)
- [ ] Chainable AI transforms
- [x] File transcription (drag + drop, symphonia: mp3/wav/flac/ogg/m4a/mp4/mkv/etc, VAD chunking)
- [x] Export options (TXT, SRT, JSON, CSV)
- [ ] Portable mode

### v1.0 - Launch ("It's ready")
- Full UX polish pass
- Landing page + website
- Distribution: winget, brew, AUR, direct download
- Documentation

### v2.0 - Agent ("It's the future")
- MCP integration
- Agent mode
- Meeting mode + diarization
- Calendar integration
- Smart commands

## 11. Open Questions

1. **First-run experience:** Bundle Moonshine Tiny (31MB) so it works immediately, or make user download a model first? Recommendation: bundle Tiny, prompt to upgrade.
2. **Overlay design:** Pill in corner vs centered bar vs macOS-style notch widget? Needs prototyping.
3. **Branding/logo:** "Inkwell" evokes ink + pen. Classic fountain pen nib? Ink drop? Quill?
4. **Landing page:** when to build? Alongside v0.1 or after?
5. **Distribution channels:** GitHub releases + direct download for v1? Package managers later?
6. **Update mechanism:** Tauri has built-in updater. Self-hosted update server or GitHub releases?

---

*This document is the source of truth for what Inkwell is. Everything else (architecture, design, code) derives from this.*
