# Security Policy

## Supported versions

The latest release only. This is a solo-maintained free project, so there are no backports.

| Version | Supported |
| ------- | --------- |
| latest release | Yes |
| anything older | No |

## Reporting a vulnerability

**Do not open a public issue for a security vulnerability.**

Preferred: use GitHub's private vulnerability reporting on this repo (Security tab > Report a vulnerability). It is private and needs no email round trip.

Alternative: email **mattias@aitappers.io**.

What to expect: acknowledgement within a few days, an honest assessment of whether and when it will be fixed, and credit in the release notes unless you would rather stay anonymous. No bounty, this project has no revenue.

## What Inkwell does with your data

Stated plainly, because a dictation app deserves specificity:

**Stays on your machine, always:**
- Audio. Captured with cpal, resampled, run through a local ONNX model. It is never uploaded and never written to disk in the normal path.
- Transcripts. Stored in a SQLite file in your OS app-data directory.
- Settings, snippets, dictionary entries and voice commands. Plain JSON files next to that database.
- API keys. Stored in the OS keyring (macOS Keychain, Windows Credential Manager, Linux Secret Service), never in a settings file.

**Leaves your machine only if you enable it:**
- **AI polish.** Off by default and gated behind a consent screen. When on, the transcribed **text** (never audio) is sent from your machine directly to the provider you configured, with your own key. There is no Inkwell server in the path. Earlier versions offered a free tier proxied through a maintainer-run Cloudflare Worker. That is removed.
- **Model downloads.** Fetched from Hugging Face and the sherpa-onnx release assets when you choose a model.
- **Update checks.** The Tauri updater asks an update endpoint whether a newer version exists. Updates are verified against a minisign public key pinned in the app, so a compromised update server cannot ship you a malicious build.

**Does not exist at all:** telemetry, analytics, crash reporting, accounts, license checks, payment processing.

## Scope

Taken seriously:

- Anything that could cause audio, transcripts or API keys to leave the machine unintentionally
- Anything that could serve or accept a malicious update
- Crafted audio or video files that cause memory corruption or code execution during file transcription
- Model downloads that could be tampered with in transit or write outside the models directory
- Webview capability or CSP weaknesses that widen what the frontend can reach

Out of scope: the app is unsigned, so anyone with write access to your machine can tamper with it. That is a known state, not a report. Same for the missing checksum verification on model downloads, which is already tracked in [TODO.md](TODO.md).
