# Security Policy

## Supported versions

The latest 1.x release only, on the Mac and on Windows. This is a solo-maintained free project, so there are no backports.

| Version | Supported |
| ------- | --------- |
| latest 1.x release (Mac: Apple silicon, macOS 26 or later; Windows: x64, Windows 11 24H2 or later) | Yes |
| older 1.x releases | No |
| 0.2.x, including the last builds for Intel Macs and Linux | No |

## Reporting a vulnerability

**Do not open a public issue for a security vulnerability.**

Use GitHub's private vulnerability reporting on this repo: **Security tab > Report a vulnerability**. It is private, it reaches the maintainer directly, and it needs no email address to exist anywhere public.

That is the only channel. There is deliberately no email address here: a published address on a public repository is scraped within days, and the GitHub route is both private and easier to act on.

What to expect: acknowledgement within a few days, an honest assessment of whether and when it will be fixed, and credit in the release notes unless you would rather stay anonymous. No bounty, this project has no revenue.

## What Inkwell does with your data

Stated plainly, because an app that listens deserves specificity:

**Stays on your machine, always:**
- Audio. Speech recognition, voice detection and speaker labels run locally. A dictation's audio is transcribed and never kept. A meeting's audio, your microphone and the call as two streams, is written to disk as it is recorded, because the final pass, crash recovery and the record's playback read it from there. It stays in the data folder with its record until the retention setting removes it (by default, records are kept).
- Transcripts, notes, summaries, commitments and settings. A SQLite database in the same folder: `~/Library/Application Support/Inkwell` on the Mac, `%LOCALAPPDATA%\Inkwell` on Windows. Nothing syncs.
- Your calendar, on the Mac, if you allow it: read to show your next meeting and to name each meeting and who was in it.
- Logs. Transcripts, notes and prompts never reach a log or an error message.

**Language models, only with your OK:**
- Polish, voice edit, meeting summaries and Ask each send words to a language model, so each is off until you allow it, feature by feature, for a named destination. On the Mac that destination is Apple's on-device model (Foundation Models, with Apple Intelligence): the words stay on the Mac.
- A consent covers the model it was given for. If a feature's model would change, it gets nothing until you agree again.
- Local-only mode, on by default, refuses any language-model endpoint that is not on this machine, in code.

**Leaves your machine:**
- **Model downloads.** The speech and voice-detection models are fetched from Hugging Face and GitHub the first time they are needed, each from a pinned revision and checked against its hash before it is used.
- **Update checks, on the Mac.** Sparkle reads the update feed of this repository's latest release, and only once you have said yes to its question. A check sends no system profile. An update is installed only if its archive's EdDSA signature matches the key the installed app carries (checked before the archive is unpacked), the feed's own signature does too, and the new app is signed by the same Developer ID team.
- **Updates, on Windows.** They come from this repository's GitHub releases. Windows builds are not code signed yet, so an update is only as trustworthy as the release it comes from.

**Does not exist at all:** telemetry, analytics, crash reporting, accounts, license checks, payment processing.

## Scope

Taken seriously:

- Anything that could cause audio, transcripts, notes or summaries to leave the machine unintentionally
- A language-model feature that runs without its consent, or reaches a model its consent does not cover, or a way past local-only mode
- Anything that could serve or accept a malicious update
- Model downloads that could be tampered with in transit, pass their hash check with the wrong file, or write outside the models directory
- Transcripts, notes or prompts reaching a log or an error message

Out of scope: Windows builds are not code signed yet, so anyone who can replace the installer before you run it can tamper with it. That is a known state, not a report.
