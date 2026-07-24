# Voice Commands - Research

## Detection Approach
Two layers:
1. **Wake word prefix** (primary): "Hey Inkwell" / "Inkwell," followed by command. Post-STT string matching on transcribed text. Zero extra dependencies.
2. **Mode toggle** (secondary): hotkey switches to command mode where all speech = commands.

No separate wake word engine needed. STT already runs. Just scan transcribed text for trigger phrases with fuzzy matching ("inkwell", "ink well", "inc well").

## Dedicated Wake Word Engines (if needed later)
- **Picovoice Porcupine**: Rust SDK (`pv_porcupine`), ~5MB model, <1% CPU, free tier
- **openWakeWord**: ONNX-based, no native Rust SDK
- Start with string matching, graduate to Porcupine only if detection issues arise.

## Command Action Types
| Type | Example | Risk |
|---|---|---|
| `open_url` | "open Google" | Moderate |
| `open_app` | "launch Notepad" | Moderate |
| `switch_model` | "use whisper" | Safe |
| `change_style` | "formal mode" | Safe |
| `insert_template` | "type email header" | Safe |
| `toggle_feature` | "pause dictation" | Safe |
| `undo` | "scratch that" | Safe |
| `custom_script` | "run my macro" | Dangerous |

## Data Model
- `VoiceCommand`: id, triggers (vec of phrases), action (enum), risk_level, enabled, confirmation_required
- `CommandAction` enum: OpenUrl, OpenApp, SwitchModel, ChangeStyle, InsertTemplate, ToggleFeature, Undo, Custom
- Stored as JSON in app data dir

## Security (Critical)
- **Whitelist-only execution.** No arbitrary shell commands.
- **Tiered confirmation:** Safe = no confirm, Moderate = 2s toast with cancel, Dangerous = modal
- **No command chaining from voice.** One command per utterance.
- **Confidence gating** (0.85 threshold). Below = "Did you mean?" prompt.
- **Rate limiting:** max 1 command per 2 seconds.
- **Audit log:** every command execution logged locally.

## Lessons from Others
- **Siri Shortcuts**: Fixed intent types with typed params. Confirmation for destructive actions.
- **Alfred**: Explicit keyword prefixes. Users learn fast. Don't try to be "smart."
- **Raycast**: Searchable command palette. Self-documenting with descriptions.

## Rust Crates
- URL opening: `tauri-plugin-opener` (official Tauri way)
- Process launching: `std::process::Command` (standard library)
- Pattern matching: `aho-corasick` for multi-pattern trigger scanning
