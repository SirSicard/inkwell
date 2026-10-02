<div align="center">

# Inkwell

**Local-first dictation and meeting notes for Mac and Windows. Free, open source, no account.**

[![Release](https://img.shields.io/github/v/release/SirSicard/inkwell?style=flat-square&color=0969da)](https://github.com/SirSicard/inkwell/releases/latest)
[![Core](https://img.shields.io/github/actions/workflow/status/SirSicard/inkwell/core.yml?style=flat-square&label=core)](https://github.com/SirSicard/inkwell/actions/workflows/core.yml)
[![Downloads](https://img.shields.io/github/downloads/SirSicard/inkwell/total?style=flat-square&color=1a7f37)](https://github.com/SirSicard/inkwell/releases)
[![License](https://img.shields.io/github/license/SirSicard/inkwell?style=flat-square)](LICENSE)
[![Platforms](https://img.shields.io/badge/macOS%20%7C%20Windows-lightgrey?style=flat-square)](#install)

</div>

**Your voice, your words, your machine.**

Inkwell does two jobs. **Dictation:** hold a key, speak, let go, and the text lands in whatever app you were typing in. **Meeting notes:** your microphone and the other side of the call are recorded as two streams, transcribed live, transcribed again properly when the call ends, and kept as a record you can search. Speech recognition runs on your own machine, so your audio never leaves it.

Inkwell is free and stays free. MIT licensed, no paid tier, no license keys, no accounts, no telemetry.

Inkwell 1.0 is a rebuild: native on each system (SwiftUI and AppKit on the Mac, WinUI 3 on Windows) over one Rust core. It replaces Inkwell 0.2, whose code lives on the [`legacy/0.2`](https://github.com/SirSicard/inkwell/tree/legacy/0.2) branch.

## What it does

### Dictation

- **Hold a key, speak, let go.** The key is a modifier held on its own: Fn (Globe) on the Mac and right Ctrl on Windows until you pick another. The text is typed into the focused app.
- **The Drop.** A small window that shows the words as you say them, then gets out of the way. It never takes the focus from the app you are typing in.
- **Modes.** How the text is written can follow the app you are typing into.
- **Snippets.** Trigger phrases that expand to full text.
- **Polish and voice edit (optional, off by default).** Polish tidies a dictation before it is typed. Voice edit, on a key of its own, rewrites the text you selected to a spoken instruction. On the Mac both run on Apple's on-device model; on Windows, on a provider you set up with your own API key. Each asks for your OK before its first use.

### Meetings

- **Offered, never started for you.** When an app has held the microphone for a few seconds, Inkwell offers to record. Nothing records until you say so, and "Record now" starts one by hand.
- **Two streams.** Your microphone is you; the call's audio is them. Echo cancellation runs on your side only when there is echo to remove.
- **Live, then final.** A live transcript while the call runs, with notes you can add; when it ends, a final pass replaces the live transcript and labels the other side's speakers when there are two or more.
- **Summary, commitments and Ask (optional).** With your OK, a language model (Apple's on-device one on the Mac, the provider you set up on Windows) writes a summary with the decisions and actions it cites, keeps the promises made in the call in **Owed**, and answers questions about the meeting.
- **Crash recovery.** Audio is written to disk as it arrives, so a crash loses seconds, not the meeting.

### The library

- **Every dictation and meeting**, newest first and searchable. A meeting's record shows its transcript, notes and summary, and plays its audio from disk.
- **Keep records** for as long as you choose: forever (the default), or 7, 30, 90 or 365 days. Anything you imported is kept whatever you choose.
- **Coming from Inkwell 0.2?** The first run, and Settings > Voice after it, offer to bring 0.2's dictation history, dictionary, snippets and modes into the library, and its dictation key when 1.0 can listen for it. 0.2's own copy is only read, never changed.

## Privacy

- Speech recognition, voice detection and speaker labels run on your machine. Audio is never uploaded.
- Dictations keep their text, never their audio. Meetings keep their audio on disk next to the transcript, because the final pass and the record's playback read it from there.
- On the Mac, if you allow it, Inkwell reads your calendar to show your next meeting and to name each meeting and who was in it. The calendar never leaves the Mac.
- Everything lives in one folder: `~/Library/Application Support/Inkwell` on the Mac, `%LOCALAPPDATA%\Inkwell` on Windows. Nothing syncs.
- For dictation, the microphone opens when you press the key and is let go of a minute after your last take. For a meeting, it is open while you record.
- Polish, voice edit, summaries and Ask use Apple's on-device model on the Mac: your words stay on the Mac. On Windows they use a provider you set up with your own API key (OpenAI, Anthropic, Groq, OpenRouter, or an OpenAI-compatible server you name, such as one on your own PC), and the words each one works on go to that provider. The key is kept in Windows Credential Manager. Each feature asks for your OK first, and asks again if where the words would go ever changes.
- There is no telemetry, no analytics, no crash reporting and no account.
- **Every network call, listed.** Model downloads fetch pinned files from Hugging Face and GitHub when you press Download, in the first run or in Settings > Models; nothing downloads before that. On the Mac, Sparkle checks this repository's releases for updates once you have said yes to its question; a check sends nothing about your Mac. On Windows, the app checks this repository's releases for a new version only when you ask it to. On Windows, a language-model provider you set up receives the words of the features you allowed, and a fixed question when you test it. Nothing else connects.

The details, and what counts as a security issue, are in [SECURITY.md](SECURITY.md).

## Install

Builds are on the [Releases page](https://github.com/SirSicard/inkwell/releases).

### Mac (Apple silicon, macOS 26 or later)

1. Download the `.dmg`, drag Inkwell to Applications, and open it. It is signed with a Developer ID and notarized by Apple, so there is no warning to click past.
2. Onboarding asks for what each job needs: **Microphone** for both, **Accessibility** for the dictation key and for typing the text, and **System Audio Recording** for the other side of a call. **Calendars** is optional: it lets Inkwell show your next meeting and name each meeting and who was in it.

Intel Macs are not supported by 1.0: Inkwell 0.2 stays the last version for them, on the Releases page.

> [!NOTE]
> Fn is also macOS's emoji and dictation key. If pressing it opens something else, set "Press 🌐 key to" to "Do Nothing" in System Settings > Keyboard, or pick another key in Inkwell's Settings.

### Windows (x64, Windows 11 24H2 or later)

1. Download `Inkwell_X.Y.Z_x64-setup.exe`. Your browser may say the file isn't commonly downloaded: choose to keep it.
2. Check the download (recommended): in PowerShell, `Get-FileHash -Algorithm SHA256` on the file must print the SHA-256 in the release notes, which is also in `Inkwell_X.Y.Z_windows-sha256.txt` on the same release. If it does not match, delete the file.
3. Run it. It installs for your user account alone, with no administrator and nothing system-wide, and brings the Visual C++ runtime its speech engines need beside the app, so there is no redistributable to install.
4. On its first run, Inkwell asks you to agree to Microsoft's terms for the Windows App SDK, the Windows SDK's .NET projection and the Visual C++ runtime it includes (in full in Settings > About). **Quit** leaves without starting anything.

> [!WARNING]
> The Windows installer is not code signed yet. SmartScreen will say "Windows protected your PC": click **More info**, then **Run anyway**. If **Smart App Control** blocks it, there is no way past it for one app: it runs only signed apps, and turning it off cannot be undone without resetting Windows, so wait for the signed build instead. [The homepage walks through it](https://getinkwell.vercel.app/#windows-install).

Updates come when you ask for them: Settings > About > **Check Now**. Inkwell 0.2 stays installed beside 1.0, not replaced by it, so import its history before you uninstall it with its application data.

Windows on ARM64 is not supported by 1.0. An ARM64 build is planned for 1.0.1.

### Linux

Not supported by 1.0. Inkwell 0.2 stays the last version for Linux, on the Releases page.

## Quick start

1. Open Inkwell and finish onboarding. Its Models step lists each model with its licence, size and source: about 3.1 GB in all on the Mac and 3.3 GB on Windows, the main speech model 2.5 GB of it. Nothing downloads until you press **Download**; you can go on while it runs, or download later in Settings > Models. The models are never inside the installer.
2. Hold the dictation key (Fn on the Mac, right Ctrl on Windows), speak, and let go. The text is typed where your cursor is.
3. Join a call. When the Drop offers to record it, say yes, or start one yourself with **Record now** on Today.

## Models

Chosen by measuring word error rates on public, human-labelled recordings (AMI meetings and FLEURS English). They download when you press Download, from a pinned revision, and are checked against a hash. The full list and the licences are in [docs/MODEL-WEIGHTS.md](docs/MODEL-WEIGHTS.md).

| Job | Model | Runs on |
|---|---|---|
| Dictation and meeting transcripts | Qwen3-ASR 1.7B | llama.cpp: Metal on the Mac, Vulkan (or the CPU) on Windows |
| Live words, and on Windows the dictations of a PC without a GPU | Parakeet TDT 0.6B v3 | FluidAudio on the Mac's Neural Engine; sherpa-onnx on the CPU on Windows |
| The other side's speakers | Nemotron-3-Diarization | NeMo-Speech.cpp: Metal on the Mac, Vulkan (or the CPU) on Windows |
| Voice detection | Silero VAD | tract |

## Support the project

Inkwell is free and will not be paywalled. If it saves you time, you can leave a tip:

> **https://buymeacoffee.com/mattiasherzig**

Suggested: EUR 10. Entirely voluntary, nothing in the app is gated behind it.

Non-financial help is worth more: file a bug, report what breaks on your hardware, or send a PR.

## Build from source

See [CONTRIBUTING.md](CONTRIBUTING.md) for the prerequisites, the checks and the codebase layout, and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the rules the code keeps.

## Requirements

- A Mac with Apple silicon on macOS 26 or later, or an x64 PC on Windows 11 24H2 or later
- Disk for the models (about 3.1 GB on the Mac, 3.3 GB on Windows), plus the meetings you keep
- A microphone

## Contributing

Bug reports and PRs are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) first. Security issues go through [SECURITY.md](SECURITY.md), not the public issue tracker.

## License

MIT. See [LICENSE](LICENSE). Third-party code and its licences are listed in [THIRD_PARTY.md](THIRD_PARTY.md), and each app's About screen carries their notices.

## Code signing policy

Free code signing provided by [SignPath.io](https://signpath.io), certificate by [SignPath Foundation](https://signpath.org).

- Committers and reviewers: [SirSicard](https://github.com/SirSicard)
- Approvers: [SirSicard](https://github.com/SirSicard)

Only artifacts built by this repository's GitHub Actions workflows from this repository's own source are signed. Each release signing request is approved by hand.

**Privacy policy:** see [Privacy](#privacy). This program does not send your audio anywhere. It contacts networked systems only to check for updates, to download the speech and voice-detection models it runs on, and, on Windows, to reach a language-model provider you set up yourself.

Status: Windows builds are not signed yet.

## Credits

Built by [Mattias Hjemgaard](https://github.com/SirSicard). Inkwell 0.2 was originally based on [Handy](https://github.com/cjpais/Handy) by CJ Pais. Speech recognition by [llama.cpp](https://github.com/ggml-org/llama.cpp) running Qwen3-ASR, live words by [FluidAudio](https://github.com/FluidInference/FluidAudio), speaker labels by [NeMo-Speech.cpp](https://github.com/NVIDIA/NeMo-Speech.cpp), and voice detection by [Silero VAD](https://github.com/snakers4/silero-vad).
