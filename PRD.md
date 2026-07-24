# Inkwell - Product Definition
*v1.0 | 2026-07-24 | Mattias Herzig*
*Supersedes PRD v0.3 (2026-03-29), which described a closed-source product with a future premium tier. That is no longer the plan.*

---

## 1. What Inkwell is

A desktop app that turns speech into text anywhere on your computer. Hold a hotkey, speak, release, and the text is pasted into whatever app has focus. Recognition runs locally on your own machine.

**One-liner:** "Your voice, your words, your machine."

## 2. What Inkwell is not

- Not a subscription. Not a trial. Not freemium.
- Not a cloud service. There is no Inkwell account and no Inkwell server that your audio or text passes through.
- Not a meeting recorder, not a transcription service, not an agent platform.
- Not an enterprise product. No teams, no shared dictionaries, no compliance features, no admin console.

## 3. Business model

**Free forever, open source, donation supported.**

- MIT licensed, public repo. Anyone can read it, fork it, build it.
- No paid tier, no license keys, no activation, no payment infrastructure, no usage limits.
- Optional AI polish is bring your own key. The user pays their own LLM provider directly. Inkwell brokers nothing.
- A voluntary donation link (suggested EUR 10) is the only ask. Nothing in the app is gated behind it, and the app never nags.

The consequence of that choice, stated plainly: there is no revenue line to fund code signing certificates, a domain, or hosted infrastructure. Those are the maintainer's costs. Anything that would create a recurring bill (a proxy, a hosted API, a license server) is out of scope by definition.

## 4. Target user

**Primary: people who type a lot and would rather not.** Writers, marketers, managers, students, developers writing prose. They want dictation that works immediately and is not creepy about it.

They are privacy conscious rather than paranoid: "local" is a feature they will pick a product for, not a policy they will audit. They will not tolerate a UI that looks like a science project.

**Secondary: power users.** They will find Advanced Mode and want snippets, voice commands, custom dictionary entries, model choice and per-app behavior.

There is no tertiary segment. Teams and enterprise are explicitly out.

## 5. Principles

1. **Local by default, and provably so.** All speech recognition runs on device. The only outbound traffic is model downloads, update checks, and BYOK polish when the user turns it on. No telemetry, ever, not even opt-in analytics until there is a good reason and an obvious switch.
2. **Beautiful by default.** The ink identity (WebGL shader panel, ink drop overlay, copper accent, one motion vocabulary) is the product's face and is not up for negotiation.
3. **Works in a minute.** Install, grant permissions, pick a model, dictate. The permission and model-download steps are real and cannot be wished away, so onboarding should make them fast and explain them rather than hide them.
4. **Progressive disclosure.** Default mode stays small. Advanced Mode unlocks the power surface.
5. **Nothing that creates a bill.** See section 3.
6. **Honest about gaps.** Documentation states what is not built. No feature claimed before it works.

## 6. Platforms

- **macOS (Apple Silicon): primary.** Developed and tested here. Needs the microphone usage description, an Accessibility permission flow, bundle-ID app detection, and eventually signing and notarization.
- **Windows 10/11: secondary.** Built in CI, shipped, not the daily driver.
- **Linux: best effort.** CI produces artifacts. Bugs get fixed if someone reports them with a repro.

## 7. Feature set (what exists)

### Core loop
Global hotkey (push to talk or toggle) -> microphone capture (cpal) -> resample (rubato) -> optional Silero VAD silence trimming -> local transcription (sherpa-onnx) -> voice command detection -> style formatting -> custom dictionary -> snippet expansion -> optional AI polish -> clipboard plus synthetic paste -> persisted to history.

### Recognition
Local models via sherpa-onnx, CPU inference, downloaded and switched in the app. Parakeet V3 is the default. The catalog currently holds 13 models, which is too many for the target user, and is scheduled to shrink to a curated three or four.

### Text processing
- **Style**: Formal, Casual, Relaxed.
- **Custom dictionary**: replacement pairs for words the model gets wrong.
- **Snippets**: trigger phrases with `{date}`, `{time}`, `{clipboard}` interpolation.
- **Voice commands**: wake prefix plus whitelisted actions.
- **Per-app style overrides**: Windows only today, macOS pending.

### AI polish (optional, off by default)
BYOK against OpenAI, Groq, Anthropic, OpenRouter or a custom OpenAI-compatible endpoint. Key in the OS keyring, never in `settings.json`. Sends text, never audio. A consent screen states exactly what leaves the machine before it is ever enabled.

### Surfaces
System tray (the app's real home), transparent always-on-top recording overlay, main window with transcript history, search, inline edit and export (TXT, SRT, JSON, CSV), file transcription by drag and drop, five-step onboarding wizard.

## 8. Roadmap shape

Direction only, no dates. Detail lives in [TODO.md](TODO.md).

1. **macOS platform pass.** Permissions, native input surfaces, kill the Windows residue. The unlock for everything else.
2. **Debt strangling.** Staged pipeline service, split the command god-file, single model registry, coarse services instead of 22 mutexes. The 60 pipeline tests stay green throughout.
3. **UI rehaul.** Sidebar information architecture, one settings store, one error path, a real type scale, real audio in the overlay bars.
4. **Streaming spike.** Partial text while you speak is the one feature gap that users actually notice. It needs an engine decision first, since the default model cannot stream.
5. **Distribution.** Signing and notarization, a real domain for the updater, Homebrew cask.

Explicitly not on the roadmap: meeting mode, diarization, calendar integration, agent mode, chainable transform chains, portable mode, GPU plumbing, mobile, browser extension.

## 9. Success metrics

The old metrics were launch-day performance numbers. The honest ones for a free project:

- Someone other than the maintainer installs it, dictates, and comes back the next day.
- Bug reports arrive with real hardware in them. The v0.1 launch got roughly 35 downloads and zero issues, which is not a signal, it is silence.
- Install to first successful transcription stays under a couple of minutes including permission grants and model download.
- The core loop never regresses: 60 pipeline tests green, dictation works with every network call disabled.

## 10. Open questions

1. **The name.** "Inkwell" collides with an Apple feature and with existing apps. Worth validating before buying a domain and a signing identity.
2. **Model catalog.** Which three or four survive the cut, and does a Whisper multilingual option stay for non-European languages?
3. **Streaming engine.** Streaming Zipformer, Moonshine v2, or a macOS-only Apple engine sidecar. Needs a spike, not an opinion.
4. **Donation platform.** Buy Me a Coffee versus GitHub Sponsors versus both. `DONATION_URL` is a placeholder until this is decided.

---

*This document defines the product. Architecture derives from it, see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).*
