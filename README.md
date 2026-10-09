<div align="center">

<img src="design/icon/icon-mac.svg" alt="" width="112" height="112">

# Inkwell

**Local dictation and meeting notes for the Mac and Windows. Free, open source, no account.**

[![Release](https://img.shields.io/github/v/release/SirSicard/inkwell?style=flat-square&color=0969da)](https://github.com/SirSicard/inkwell/releases/latest)
[![Core](https://img.shields.io/github/actions/workflow/status/SirSicard/inkwell/core.yml?branch=main&style=flat-square&label=core)](https://github.com/SirSicard/inkwell/actions/workflows/core.yml)
[![Downloads](https://img.shields.io/github/downloads/SirSicard/inkwell/total?style=flat-square&color=1a7f37)](https://github.com/SirSicard/inkwell/releases)
[![License](https://img.shields.io/github/license/SirSicard/inkwell?style=flat-square)](LICENSE)
[![Platforms](https://img.shields.io/badge/macOS%2026%20%7C%20Windows%2011-lightgrey?style=flat-square)](#install)

</div>

Inkwell does two jobs. **Dictation**: hold a key, speak, let go, and the text lands in the app you
were typing in. **Meeting notes**: your microphone and the other side of the call are recorded as
two streams, transcribed while you talk and again when the meeting ends, and kept in a library you
can search. Speech recognition runs on your own machine.

Inkwell 1.0 is a native app on each platform over one Rust core:

- **Mac:** macOS 26 or later, Apple silicon. SwiftUI and AppKit.
- **Windows:** Windows 11 24H2 or later, x64. WinUI 3.

It is MIT licensed, with no paid tier, no licence keys, no account and no telemetry of its own.

Inkwell 0.2, the earlier Tauri app, is kept on the [`legacy/0.2`](https://github.com/SirSicard/inkwell/tree/legacy/0.2)
branch. Intel Macs and Linux get no 1.0: Inkwell 0.2 is their last version.

## What it does

### Dictation

- **Push to talk, anywhere.** The dictation key can be a modifier (or Fn on the Mac) held on its
  own, a function key, or modifiers and one key. A left-hand modifier waits a moment before
  dictation starts, so its shortcuts keep working. Recording a shortcut in
  Settings > Dictation checks it on the spot and says why when a key can't be used.
- **Live words while you speak**, shown in the Drop, the small recording light. They are
  provisional; the text that is typed comes from the final pass when you let go.
- **The 300 ms before you press** are kept while the mic is open (it closes after a minute
  without a take), so the first word isn't clipped. A hold that never ends (a lost key release)
  is stopped after 180 seconds and processed, never thrown away.
- **Voice editing.** Select text, hold the edit key, say what to change, and the rewrite replaces
  the selection. It needs a language model ([below](#language-models-optional)).
- **Styles, cleanup, snippets and voice commands.** Formal, Casual or Relaxed casing and
  punctuation; filler words and stutters removed; trigger phrases that expand to text; spoken
  commands. Each is edited in Settings.
- **Polish (optional, off until you agree).** A language model tidies grammar and false starts.
  When its answer adds to what you said, or carries out a dictated instruction instead of writing
  it down, the dictation goes in as said, with a warning.

### Meetings

- **Both sides, as two streams.** Your microphone is you; the other side is captured from the
  meeting app (on the Mac, a process tap of that app, else of everything the Mac plays except
  Inkwell; on Windows, process loopback for Zoom and the browsers, else the output device the app
  plays to). Echo cancellation runs on the mic only, and only when there is an echo to cancel.
- **A live transcript, then a final pass.** At the end the meeting is transcribed again with the
  more accurate model, and the final pass replaces the live one in one step. It is refused if it
  comes back empty or with fewer than half the words on either side.
- **Who said what on the far end.** The diarizer labels the far end's speakers when there are at
  least two; you can name or rename them in the record.
- **Summary and commitments**, when a language model is available: the summary keeps a citation
  to the line each decision and action came from, and each promise names who it is owed to.
- **Recording is your decision.** When an app has held the microphone for three seconds, Inkwell
  follows that app's choice: **Ask** (offer to record), **Always** or **Never**, set from the Drop
  or Settings > Meetings. Ask is the default. The first time Always records a call, the Drop
  reminds you to tell the others. For the first minute of a meeting, **Stop and delete** ends it
  and deletes it as if it had never been made.
- **Notes while you talk** on the Live screen, beside the transcript, and a global shortcut to
  start and stop a meeting, set in Settings.
- **Crash recovery.** Audio is written to disk as it is recorded. After a crash, the meeting is
  repaired and finished from what is on disk.
- **Calendar (Mac).** With calendar access, Today shows your next meeting, and a meeting is named
  after its calendar event and who was in it. The calendar is not available in Inkwell 1.0 for
  Windows: meetings there are named by the app they were in.

### The Library, search and Ask

- **The Library** lists every dictation and meeting, newest first, filtered by kind or searched
  through everything that was said.
- **A record** opens with your notes beside the transcript, the summary, and what is owed, with a
  player for both sides on one timeline. Every timestamp and line plays from its moment.
- **Ask**, during a meeting: type a question about what has been said so far and a language model
  answers from the transcript. The far end's questions to you are stacked above it.
- **Retention.** Keep everything (the default), or delete meetings and dictations older than 7,
  30, 90 or 365 days. A delete overwrites the text in the database and removes the audio.

### Owed

Owed lists the promises still open across your meetings, grouped, with overdue ones marked. Mark
one done (with an Undo for a few seconds), or play the moment it was said. When a later meeting
says an open promise is already done, Owed suggests it "looks done" for you to confirm.
Commitments come from the meeting's summary, so they need a language model.

### Stats

Counted from your library: words dictated, speed against your own past, time saved over typing
(with the assumption printed beside it), a streak and a heatmap; meeting hours, talk time by side,
the longest monologue and your questions; promises kept; milestones; and a weekly review. A share
card holds only the numbers you tick, to copy or save.

### Modes

A mode bundles a style, filler removal and polish (with its own instructions, and optionally its
own language model), and switches on by the app you are typing in: the first mode whose app list
matches the frontmost app wins, else the default. Apps are matched by bundle id on the Mac and by
executable name on Windows. Modes are edited in Settings > Modes.

## Privacy

- **Speech recognition runs on your machine.** Audio is captured, transcribed and stored locally.
  It is never uploaded.
- **The library is a local SQLite file**, with meetings' audio beside it: on the Mac in
  `~/Library/Application Support/Inkwell`, on Windows in `%LOCALAPPDATA%\Inkwell`. Nothing syncs.
- **Local only is on unless you turn it off.** While it is on, the core refuses any language-model
  endpoint that is not on this machine, in code.
- **Every feature that sends text asks first.** Polish, voice edit, meeting summaries and Ask each
  ask where your words will go (this machine, or one named provider) and send nothing until you
  agree. If the model a feature uses moves to another destination, it sends nothing until you
  agree again. Only text is sent, never audio.
- **Keys** are kept in the macOS Keychain or the Windows Credential Manager, never in a settings
  file, and go from your machine straight to the provider you chose. No server belonging to this
  project is in the path.
- **Logs never contain what you said.** Transcripts and prompts are kept out of log lines and error
  messages.
- **What connects to the network:** model downloads (from Hugging Face, at pinned revisions,
  checked against their SHA-256 before use), update checks against this repository's GitHub
  releases, and the language-model provider you set up with your own key, if any.
- **Updates.** On the Mac, Sparkle asks before its first automatic check, and a check sends no
  system profile. On Windows, Inkwell checks only when you press Check Now in Settings > About.
- **Windows App SDK.** The Windows app ships Microsoft's Windows App SDK runtime, which takes part
  in Windows' own diagnostic data, controlled in Settings > Privacy & security > Diagnostics &
  feedback. It never sees what you dictate or record. Details:
  [windows/WINDOWS-APP-SDK-TELEMETRY.md](windows/WINDOWS-APP-SDK-TELEMETRY.md).

## Install

Builds are on the [Releases page](https://github.com/SirSicard/inkwell/releases/latest).

### Mac

1. Download `Inkwell_X.Y.Z_aarch64.dmg`, open it and drag Inkwell to Applications. The app is
   signed with a Developer ID, and the app and the disk image are notarized by Apple, so it opens
   without a warning.
2. Open Inkwell. It lives in the menu bar; the first run walks through the permissions, the speech
   models, the import from 0.2 and the optional language model.

### Windows

Inkwell for Windows 11 (24H2 or later), x64. It installs for your user account alone: no
administrator password, nothing system-wide.

1. **Download** `Inkwell_X.Y.Z_x64-setup.exe` from the release. Your browser may say the file
   isn't commonly downloaded; choose to keep it.
2. **Check the download** (recommended). The Windows build is not code-signed yet, so this is how
   you know the file is the one that was published. In PowerShell:

   ```powershell
   Get-FileHash -Algorithm SHA256 "$HOME\Downloads\Inkwell_X.Y.Z_x64-setup.exe"
   ```

   The hash must match the one in the release notes and in `Inkwell_X.Y.Z_windows-sha256.txt` on
   the same release. If it does not, delete the file and do not run it.
3. **Run it.** SmartScreen says "Windows protected your PC", with an unknown publisher. Choose
   **More info**, then **Run anyway**. Inkwell installs and opens, and is in the Start menu.

> [!WARNING]
> If **Smart App Control** blocks the installer, there is no "Run anyway": Smart App Control does
> not run unsigned apps, and Microsoft offers no per-app exception. Wait for a signed build rather
> than turning off a protection you chose to keep.

The first run shows Microsoft's terms for the Windows App SDK, the Windows SDK's .NET projection
and the Visual C++ runtime that Inkwell includes, with Agree and Quit, before anything else starts.

**Uninstalling:** Settings > Apps > Installed apps > Inkwell > Uninstall. Your library (meetings,
dictations, notes and downloaded models) stays in `%LOCALAPPDATA%\Inkwell`; delete that folder too
to remove it.

### Coming from 0.2

The 0.2 app's updater cannot install 1.0, so download 1.0 from the release. The first run, and
Settings, offer to import 0.2's data: your dictation history, dictionary, snippets, voice commands,
modes and settings, with your stored API keys linked rather than copied. The
Homebrew cask still installs 0.2 for now.

## Permissions

| What Inkwell asks for | Mac | Windows | Without it |
|---|---|---|---|
| Hear you | Microphone | Microphone | Nothing you say can be written down |
| Hear the others | System audio | No permission to grant | Meetings record only your voice |
| Type for you | Accessibility (also needed for the dictation key) | No permission to grant | Dictation can't type into other apps or hear the dictation key |
| Know your meetings | Calendar (optional) | Not available in 1.0 | Meetings are named by the app they were in |

A permission check never brings up a prompt on its own: Inkwell asks when you press the button for
it, and checks again when you come back from the system's settings.

Known limits:

- **Mac:** while Secure Input is on (in a password field, for example), nothing is typed. The Drop
  says so, and your words are in the Library.
- **Windows:** an app running as administrator can't be typed into, and its keys can't be heard.

## Models

Inkwell never bundles model weights. It downloads each when it is first needed, from a pinned
revision with a checked hash. The speech models were chosen by measuring word error rate on public
human-labelled English sets (AMI meetings, FLEURS English); Inkwell 1.0 targets English.

| Job | Model | Runs on | Licence | Size (approx.) |
|---|---|---|---|---|
| Dictation and meeting final pass | Qwen3-ASR 1.7B, Q8_0 | llama.cpp: Metal on the Mac; Vulkan, or the CPU, on Windows | Apache-2.0 | 2.5 GB |
| Live words (Mac) | Parakeet TDT 0.6B v3, Core ML | FluidAudio, on the Neural Engine | CC-BY-4.0 | 0.48 GB |
| Live words (Windows), and dictation on a PC without a GPU | Parakeet TDT 0.6B v3, int8 ONNX | sherpa-onnx, on the CPU | CC-BY-4.0 | 0.67 GB |
| Who said what on the far end | Nemotron-3-Diarization, q8_0 | NeMo-Speech.cpp | OpenMDW-1.1 | 0.11 GB |
| Voice activity | Silero VAD v6.2.3 | tract (pure Rust) | MIT | 1.3 MB |
| Polish, voice edit, summaries and Ask on this PC (Windows only, optional) | Qwen3-4B-Instruct-2507, Q4_K_M | llama.cpp | Apache-2.0 | 2.5 GB |

The first run offers them by what they do: the set every job needs (voice detection and Parakeet),
**Fewer mistakes** (Qwen3-ASR) and **Tell the people on the call apart** (the diarizer). Nothing
downloads until you press Download, and models can be added or removed later in Settings > Models.
On Windows, a meeting keeps its live transcript until Qwen3-ASR is installed.

Credits: Parakeet TDT v3 and Nemotron-3-Diarization are by NVIDIA; Qwen3-ASR and
Qwen3-4B-Instruct-2507 are by the Qwen team (Alibaba Cloud), the latter converted to GGUF by
Unsloth; Silero VAD is by the Silero team. Settings > About lists every model and library the app
ships with its licence. The policy is in [docs/MODEL-WEIGHTS.md](docs/MODEL-WEIGHTS.md).

## Language models (optional)

Dictation and meeting transcripts never need a language model. Polish, voice editing, meeting
summaries and commitments, and Ask do. There are three ways to get one:

- **Mac: Apple's on-device model.** With Apple Intelligence on, Inkwell uses Apple's Foundation
  Models. Nothing to download, no key, and your words stay on the Mac.
- **Windows: a model on this PC.** Qwen3-4B-Instruct-2507 (2.5 GB), downloaded only when you ask for
  it, in the first run or in Settings > AI. It runs in Inkwell on the GPU (Vulkan) or the CPU, so
  Local only stays on and no key is needed.
- **Your own key**, on either platform: OpenAI, Groq, Anthropic, OpenRouter, or any
  OpenAI-compatible server you name, such as one you run yourself. A provider you choose is used
  before the on-device model. A provider that is not on this machine needs Local only off:
  agreeing to send to it turns Local only off, and each feature asks before it sends anything.

### A free Groq key

Groq's Free plan costs $0 and has rate limits, listed on its
[Rate Limits page](https://console.groq.com/docs/rate-limits). The app's guide (Settings > AI,
"How to get a free Groq key") has the same steps:

1. Open Groq's API Keys page: [console.groq.com/keys](https://console.groq.com/keys).
2. Log in, or make a Groq account.
3. Press **Create API Key** and give the key a name, such as Inkwell.
4. Copy the key. In Inkwell's Settings > AI, choose Groq, paste it and press **Save key**, then
   **Use Groq**.

## Not in 1.0

- Linux and Intel Macs (0.2 is their last version).
- Windows on ARM64.
- A code-signed Windows build.
- The calendar on Windows.
- A Homebrew cask for 1.x.

## Build from source

The repository holds the Rust core (`core/`), the Mac app (`mac/`) and the Windows app
(`windows/`). Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) before a structural change.

**The core** (Rust; the toolchain is pinned in `core/rust-toolchain.toml`):

```bash
cd core
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

The tests need no models, GPU or audio devices: they use a mock engine and replayed fixtures.
Tests that need real models are `#[ignore]` and run locally with `INK_BENCH_DIR` set.

**The Mac app** (macOS 26 on Apple silicon, Swift 6.2; there is no `.xcodeproj`: SwiftPM builds it
and a script bundles and signs it):

```bash
mac/scripts/build-mac.sh --debug                                   # mac/build/Inkwell.app, without the bundled engines
swift test --package-path mac --only-use-versions-from-resolved-file
```

Without `INK_SIGN_IDENTITY` the build is signed ad-hoc, and macOS treats each rebuild as a new
app, so its permission grants reset; set it to a signing identity of your own to keep them.
Building with the speech engines as released (`--engines`) needs NeMo-Speech.cpp and its pinned
dependencies built first: [docs/RELEASING.md](docs/RELEASING.md), "The engines".

**The Windows app** (x64, the .NET SDK pinned in `windows/global.json`). Its tests load the core's
DLL, so build that first:

```powershell
cd core; cargo build -p ink-ffi --lib; cd ..\windows
dotnet test Inkwell.slnx
```

The release build, with its engines, needs Visual Studio's C++ build tools, CMake, Ninja, LLVM, Git
Bash and the Vulkan SDK: [docs/RELEASING.md](docs/RELEASING.md), "Inkwell 1.x on Windows".

## Contributing

Bug reports and pull requests are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) first. Security
issues go through [SECURITY.md](SECURITY.md), not the public issue tracker. Code dependencies must be
MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC or Zlib; third-party code is listed in
[THIRD_PARTY.md](THIRD_PARTY.md).

## Support the project

Inkwell is free and will not be paywalled. If it saves you time, you can leave a tip:

> **https://buymeacoffee.com/mattiasherzig**

Suggested: EUR 10. Entirely voluntary, nothing in the app is gated behind it.

Non-financial help is worth more: file a bug, report what breaks on your hardware, or send a PR.

## License

MIT. See [LICENSE](LICENSE).

## Code signing policy

Free code signing provided by [SignPath.io](https://signpath.io), certificate by [SignPath Foundation](https://signpath.org).

- Committers and reviewers: [SirSicard](https://github.com/SirSicard)
- Approvers: [SirSicard](https://github.com/SirSicard)

Only artifacts built by this repository's GitHub Actions workflows from this repository's own source are signed. Each release signing request is approved by hand.

**Privacy policy:** see [Privacy](#privacy). This program does not send your audio anywhere. It contacts networked systems only to check for updates, to download the speech, voice-detection and language models it runs on, and, when you set up a language-model provider with your own API key, to send text to that provider.

Status: Windows builds, 1.0 included, are not signed yet. The Mac app is signed with a Developer ID and notarized by Apple.

## Credits

Built by [Mattias Hjemgaard](https://github.com/SirSicard). Inkwell 0.2 was originally based on
[Handy](https://github.com/cjpais/Handy) by CJ Pais. Inkwell 1.0 runs on
[llama.cpp](https://github.com/ggml-org/llama.cpp),
[FluidAudio](https://github.com/FluidInference/FluidAudio),
[sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx),
[NeMo-Speech.cpp](https://github.com/NVIDIA/NeMo-Speech.cpp),
[Silero VAD](https://github.com/snakers4/silero-vad) on [tract](https://github.com/sonos/tract),
[Sparkle](https://github.com/sparkle-project/Sparkle) and [Velopack](https://github.com/velopack/velopack),
with the models credited [above](#models). The full list, with licences, is in
[THIRD_PARTY.md](THIRD_PARTY.md) and the app's Settings > About.
