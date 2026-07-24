# Inkwell - Roadmap

*Replaces the old pre-launch build checklist, which was complete and stale.*
*Ordered by dependency, not by date. No dates are promised.*

Source of the analysis behind this list: [docs/rehaul-analysis-2026-07-24.md](docs/rehaul-analysis-2026-07-24.md). Target architecture: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

---

## 0. Rehaul (in progress)

- [x] Delete voice agent mode (module, tab, hotkey, sounds, settings, pipeline branch)
- [x] Delete the free AI polish proxy tier and its quota tracking
- [x] Documentation, licensing and repo identity pass
- [x] Delete `inkwell-worker/` (the Cloudflare Worker that served the dead free tier)
- [ ] Shut the deployed worker down at Cloudflare. Deleting the source does not stop the running service.
- [ ] Secure the minisign updater private key outside of GitHub Actions secrets. Losing it permanently breaks the update chain for every installed copy.
- [ ] Fix `src-tauri/Cargo.toml` scaffold metadata (`description = "A Tauri App"`, `authors = ["you"]`, empty license and repository)

## 1. macOS platform pass

The unlock. Everything downstream is easier once the app is a good macOS citizen.

- [ ] `NSMicrophoneUsageDescription` in the bundle Info.plist. Without it a bundled build gets killed by TCC.
- [ ] Accessibility permission flow: check before the first synthetic paste, explain it in onboarding, link straight to the settings pane
- [ ] Kill the permanent `getUserMedia` stream in InkCanvas. A privacy-branded app must not hold the mic indicator on from launch. Drive the shader from Rust-emitted band events instead (the fallback path already exists).
- [ ] Ship `silero_vad.onnx` or download it on first run. Right now silence trimming silently no-ops because nothing fetches the file.
- [ ] Bundle-ID based app detection to replace the Windows-only process-name matching, so per-app styles work on macOS
- [ ] Default hotkey that does not collide with macOS input-source switching
- [ ] Menu bar template icon
- [ ] Mac QA checklist: TCC grant and revoke, Secure Input fields, AirPods and Bluetooth mic sample rates, multi-display overlay placement, first-run on a clean machine

## 2. Strangle the debt

Keep the 60 pipeline tests green through every step.

- [ ] Extract the dictation pipeline out of the global-shortcut closure into a staged async service
- [ ] Split `commands.rs` (913 lines) by domain
- [ ] One model registry table instead of six hardcoded match statements, plus checksums and `.partial` downloads so an interrupted download does not wedge permanently
- [ ] Collapse AppState's 22 mutex fields into a few coarse services, `spawn_blocking` for inference
- [ ] Restore the saved model at launch (setup currently hardcodes Parakeet else Moonshine and ignores `settings.model`)
- [ ] Honor `mic_device`, `start_on_boot` and `show_overlay`, which are persisted and then ignored
- [ ] Re-enable CSP and narrow the window capabilities
- [ ] Dedupe the 15s chunk overlap so words stop repeating at boundaries
- [ ] Save and restore the user's clipboard around paste
- [ ] Delete dead code: standby ring buffer, `download_parakeet`, hand-rolled calendar math, hand-written JSON

## 3. UI rehaul

- [ ] Replace 12 horizontal tabs with a grouped sidebar. Half of them hide behind an overflow menu at the default window width.
- [ ] One zustand settings store, one error-toast path. Roughly 30 empty `.catch(() => {})` sites currently swallow every failure.
- [ ] Real type scale with a 13px floor. The 40-plus arbitrary `text-[Npx]` values are the single biggest reason screenshots read as dated.
- [ ] Feed real audio into the overlay bars. They are a sine wave of time today, on the one surface the user watches during every dictation.
- [ ] Delete the ~527 dead frontend lines (unimported `App.css`, stale duplicate Onboarding/TabBar/StatusBar) and the dead deps (7 of 8 Radix packages, geist npm, one of the two loaded font families)
- [ ] Confirm before destructive actions, replace `alert()`, label the toggle dots, make hover-only buttons reachable
- [ ] Single version source. The status bar and the About tab currently disagree.
- [ ] Trim the model catalog to a curated three or four

## 4. Streaming spike

The one feature gap users actually notice. Research starting point: [docs/research/raw/08-streaming.md](docs/research/raw/08-streaming.md), with the caveat that its Streaming Zipformer assumption predates better options.

- [ ] Spike: streaming Zipformer vs Moonshine v2 vs an Apple SpeechAnalyzer sidecar. Parakeet is offline-only in sherpa-onnx, so partials need a second model either way.
- [ ] If it survives the spike: two-pass pipeline, streaming front end for instant partials, final pass rescored by the offline model
- [ ] Partial and final text rendering in the overlay

## 5. Distribution

- [ ] Apple Developer ID signing plus notarization in the existing `build.yml`
- [ ] Windows signing
- [ ] Move the updater endpoint off `workers.dev` onto an owned domain, then verify an end-to-end update on both platforms
- [ ] Homebrew cask
- [ ] Replace `DONATION_URL`'s placeholder with the real link
- [ ] Screenshot and demo GIF in the README, homepage rebuild

## Not doing

Meeting mode, speaker diarization, calendar integration, agent mode, chainable transform chains, portable mode, GPU and CUDA plumbing, mobile, browser extension, telemetry, any paid tier.
