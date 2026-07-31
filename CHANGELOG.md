# Changelog

All notable changes to Inkwell will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.2.4] - 2026-07-31

### Changed

- **Network connections now use a pure-Rust TLS stack.** Inkwell previously linked OpenSSL for its HTTPS connections, which accounted for the largest group of known vulnerabilities against the project. Nothing changes in how the app behaves; model downloads, update checks and AI polish were each verified against the real servers after the switch.

### Fixed

- Dependencies across the app, the website and the update service brought current. The website had 23 unpatched advisories against it, which mattered more than the rest because it is the only part of this project reachable from the internet.

## [0.2.3] - 2026-07-31

### Added

- **Qwen3 ASR**, the most accurate model measured here: 5.6% word error rate against 8.0% for the best previous option, on the same recordings. It is also the only model that covers English and the Nordic languages at once, so it does not force a choice between them. Larger download and about twice the transcription time, which is still under a second for a short dictation. Offered as an accuracy tier, not as the default.
- **The dictionary now applies to models that read it at load time.** Qwen3 takes its bias phrases when the engine is built rather than per dictation, so saving the dictionary rebuilds the engine. Without this, an edit would have saved successfully and silently done nothing until the next restart.

### Changed

- Updated the speech engine (sherpa-onnx 1.12 to 1.13). Existing models load and score within noise of their previous numbers.

## [0.2.2] - 2026-07-31

### Added

- **Voice editing.** Select text anywhere, hold a second hotkey (Cmd+Shift+E on macOS), say what to change, and the rewrite replaces the selection. "Make this shorter", "fix the grammar", "turn this into bullet points". Needs an API key, because rewriting text to order is a language-model job; dictation itself stays entirely local. Clear the hotkey in General to switch the feature off and free the shortcut.
- **A Troubleshooting section**, with debug recording and a button that opens the folder it writes to. That toggle used to sit in General next to the theme picker, though it is a tool you switch on to investigate something and off again, and it writes your voice to disk while it is on.

### Changed

- **The model list is four models instead of thirteen**, each answering a different question, with word error rates measured on real recordings rather than quoted from a leaderboard. Parakeet V2 is the most accurate for English (8.0%), SenseVoice is a quarter of the size and twice the speed at 9.3%, Whisper Turbo reaches the languages the others cannot, and Parakeet V3 remains the default because it detects 25 European languages for you. The models that left lost on every axis at once: smaller Whisper builds were both less accurate and slower than Parakeet, and the English-distilled ones were beaten by Parakeet V2 at a similar size.
- If your saved model is one of the retired ones, the app says so instead of quietly loading a different one.

### Fixed

- **macOS asked for permission to control the computer on every single dictation.** The input library's defaults prompt when the process is not trusted, and the paste path builds one per dictation. Worse, it fired while the Accessibility switch read as granted: macOS ties that grant to the app's code identity, and an unsigned build gets a new identity every time it is replaced, so updating the app invalidated the permission while leaving the toggle on. The paste path no longer prompts; when the permission really is missing, one message names the fix, including that an already-listed Inkwell has to be removed and re-added.
- Saying "formal mode" had done nothing since modes shipped. The command wrote a setting the pipeline stopped reading when modes took ownership of styling. Style commands now pin a mode, and the Modes tab shows the pin with a Clear button so it is never invisible.
- Debug recordings were numbered 1, 3, 5, 7, because the sequence added a per-session counter to a directory count.
- Onboarding described the app as "premium speech-to-text", a line left over from a planned paid tier that no longer exists.

## [0.2.1] - 2026-07-30

Transcription accuracy. Diagnosed from a real transcript history rather than from impressions: the microphone and the model's core acoustics were fine, and the errors traced to vocabulary, chunk seams and word edges.

### Added

- **The dictionary now biases the recogniser.** Its correction targets are fed to the decoder as hotwords, so a custom word can be recognised correctly in the first place instead of being repaired afterwards. Decoding moved from greedy to beam search to support it. Add "Inkwell" and the model stops guessing "Inco".
- **Parakeet V2 (fp16)** and **Parakeet V3 (full precision)** in the model list, for measuring what int8 quantisation costs on your own voice.
- **A model comparison tool** (`cargo run --release --example ab_models`) that scores every installed model over a corpus of your recordings by word error rate.
- Debug recordings are kept per take in `~/Documents/Inkwell Debug Audio`, so a corpus can accumulate.

### Fixed

- **The first phoneme of a dictation is no longer lost.** People start speaking as they press, so the 300ms before the keypress is now included, along with the 300ms after release for the last word's ending. The buffer that was supposed to do this had been dead code since it was written: its consumer was dropped at creation, so it filled once and silently discarded everything after.
- **Words no longer duplicate or split at chunk boundaries** in long dictations. Audio was cut at blind 15-second offsets, and because silence removal ran first, every cut was guaranteed to land inside continuous speech. Cuts now fall at the quietest moment near a 60-second mark, and the "20-second Parakeet limit" that justified the old window did not exist.
- **Pauses reach the model.** Silence removal used to delete every pause and splice the remaining speech together, which put unrelated phonemes hard against each other. It now trims only the dead air at the ends.
- **One loud click no longer ruins a whole dictation.** Recording level was set from the single loudest sample, so a keyboard press at full scale made the app conclude the take was loud enough and leave quiet speech untouched.
- The last few milliseconds of every recording stopped being silently dropped by the resampler.
- A second launch fronts the running app instead of racing it for the microphone and hotkey.
- File transcription cuts at quiet points rather than blindly at 30 seconds.

### Changed

- History records how long you held the key, rather than how much audio survived processing, so internal changes stop rewriting what past entries mean.

## [0.2.0] - 2026-07-30

Rehaul. The product is now explicitly free and open source forever, macOS first, and BYOK only.

### Added

- **Modes.** One bundle holds the text style, speech cleanup, polish prompt and the list of apps it applies to; the first mode whose apps match the frontmost application wins. This replaces three settings that used to decide independently how a dictation was written (global style, per-app style rules, global polish prompt), so "formal, polished, only in email" is expressible for the first time. Existing per-app rules migrate automatically, once.
- **Speech cleanup.** "um", "uh" and immediate stutters are stripped before pasting, on by default per mode. The stutter collapse works from a curated list of function words, because "he had had enough" and "I know that that is true" are valid English and a cleanup step must never change what a sentence says.
- **Pause and resume** during a dictation, from the tray menu. Paused audio is simply not captured; the flag clears on both start and stop so a leftover pause can never silently eat the next dictation.
- **Teach from a transcript.** The dictionary could always fix a recurring mistranscription, but asked you to predict one in advance; now you correct it from the transcript in front of you and the correction is saved.
- **Automatic language detection, surfaced.** The default model always transcribed 25 European languages without being told which one; nothing in the app or the homepage said so.
- **Grouped sidebar navigation.** Twelve horizontal tabs, half of them behind an overflow menu at the default window width, became grouped sections with an Advanced toggle.
- **Light theme**, following the system appearance. The ink panel's cream field and charcoal ink were already a complete light palette sitting inside the app doing decorative duty; the shell now inverts to match. In light mode the ink panel merges with the page, so the mark reads as ink on paper.
- **Overlay position.** Six placements instead of a hardcoded bottom-centre.
- **Dictionary CSV import.** Merges rather than replaces, splits on the first comma so replacements may contain commas, and only drops a header row when it looks like one.
- **A dedicated menu-bar icon.** The tray previously reused the 128px app icon, which macOS flattens to a solid silhouette at menu-bar size. The new asset is drawn for 22pt with the nib preserved and a margin so it does not crowd its neighbours.
- **Recording state in the status bar.** `--color-accent-recording` had been defined since the first build and used nowhere; this is what it was for.

### Removed

- **Voice Agent mode**, entirely: the second hotkey, the Agent settings tab, the pipeline branch, the agent sounds and the agent settings. It targeted the OpenClaw gateway, which was decommissioned 2026-06-15, and the tab never persisted anything anyway (it sent the wrong invoke shape and the backend had no matching handlers).
- **The free AI Polish proxy tier.** Polish previously had a "free 4,000 words per week" mode routed through a Cloudflare Worker in front of the maintainer's own Groq key. It cost money and returned nothing. AI polish is now bring your own key, or off.
- Weekly word-count quota tracking, which existed only to enforce that free tier.
- Windows-developer-machine workarounds that never belonged in a cross-platform build.

### Changed

- **macOS (Apple Silicon) is the primary platform.** Windows is built in CI and supported as a secondary target. Linux is best effort.
- Monetization is a voluntary donation link. There is no paid tier, no license key, no activation and no payment code in the app.
- Documentation rewritten to match what the code actually does, including the unsigned-installer friction on both platforms and the macOS Accessibility permission that synthetic paste requires.
- PRD rewritten as a lean product definition. The old "closed source, future premium tier" line and the Meeting-mode and Teams/Enterprise roadmap entries are gone.
- `research/` moved to `docs/research/` with a status index. Model-catalog expansion is shelved. Streaming research is retained as the starting point for a future spike.
- **Type scale.** 65 arbitrary font sizes replaced by four steps (11 / 13 / 15 / 20). Body text moves from 11px to 13px, matching the system and every competitor; the 9px and 10px labels are gone.
- **Contrast, measured in both themes.** Tertiary text was `rgba(255,255,255,0.35)`, which composited to 3.17:1 and failed the 4.5:1 AA floor in all 118 places it was used. It is now 6.28:1. Borders moved from 0.10 to 0.14, since dividers were effectively invisible.
- **The ink column no longer eats the window.** It was a flat 35% at every width; it is now capped, and its warp halved, because the 0.20 baseline was tuned for the 97px overlay and read as a splatter at 400px.
- `Glass*` components renamed `Ink*`. They described a backdrop blur they never had.
- **Internals**, for anyone reading the diff: the speech engine moved to a dedicated thread (removing an `unsafe impl Sync`), the eight duplicated model tables became one, the 913-line command file split by domain, blocking commands went async so the UI stops freezing during them, and the 22-mutex app state collapsed into a few coarse services. 132 tests, up from 60.

### Fixed

- **The app crashed on every successful dictation on macOS.** The synthetic Cmd+V used a layout-dependent keycode lookup that goes through the Text Services Manager, which asserts it is on the main thread; the pipeline pastes from a worker. Not a Rust panic, so no handler could catch it. The V key is now pressed by its raw hardware keycode.
- **Captured audio was ~35 dB too quiet** on some microphones: raw CoreAudio capture bypasses the system's input gain, and multi-mic arrays were averaged across channels, which phase-cancels the voice. Recordings are now peak-normalized and arrays contribute their primary channel.
- **Pasting could land in Inkwell itself** instead of the app you were dictating into, because showing any Inkwell window activated it. The app now retreats to menu-bar-only (Accessory) when its windows close.
- **API keys were silently never saved.** keyring 3 ships no credential store unless a backend feature is enabled, so every save failed invisibly. Platform backends are now enabled explicitly and a round-trip test guards them.
- **Opening the AI tab asked macOS for keychain access every time.** Rendering "key configured" fetched the actual secret, which macOS gates behind an authorization dialog. Whether a key exists is now answered by an attributes-only query, which the OS does not gate; the secret is read only at the moment polish uses it.
- **Denying that keychain prompt no longer sends your transcript out anyway.** A denied read used to become an empty API key and the dictation was posted to the provider regardless, to be rejected there. Saying no now means the text never leaves the machine.
- **Dark mode was unreachable** after the light theme shipped following the system with no override. Appearance is now System, Light or Dark.
- **First-run onboarding invoked a download command that no longer existed**, so a fresh install could never fetch its model; and the catalog offered a model with no download URLs. Both gone.
- **The recording overlay no longer lies.** Its level bars were a sine of elapsed time: they animated identically whether the microphone heard a voice or silence. They now follow the real `audio-amplitude` events, which is exactly the signal that would have made a 35 dB capture bug visible instead of invisible.
- Privacy: the agent token is no longer written in plaintext to `settings.json` (the code previously saved a copy there even when the OS keyring succeeded).
- Privacy: raw dictation audio is no longer dumped to a temporary WAV file on every recording in the production path.

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

[0.2.4]: https://github.com/SirSicard/inkwell/releases/tag/v0.2.4
[0.2.3]: https://github.com/SirSicard/inkwell/releases/tag/v0.2.3
[0.2.2]: https://github.com/SirSicard/inkwell/releases/tag/v0.2.2
[0.2.1]: https://github.com/SirSicard/inkwell/releases/tag/v0.2.1
[0.2.0]: https://github.com/SirSicard/inkwell/releases/tag/v0.2.0

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

