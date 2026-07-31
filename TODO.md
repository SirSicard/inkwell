# Inkwell - Roadmap

*Ordered by dependency, not by date. No dates are promised.*
*State audited 2026-07-31. v0.2.1 is the current release, all four platforms green, updater serving it. The launch is done; what follows is trust (signing), reach (Windows, Homebrew) and accuracy.*

Analysis behind the rehaul: [docs/rehaul-analysis-2026-07-24.md](docs/rehaul-analysis-2026-07-24.md). Feature research: [docs/competitive-extras-2026-07-27.md](docs/competitive-extras-2026-07-27.md). Streaming verdict: [docs/streaming-spike-2026-07-31.md](docs/streaming-spike-2026-07-31.md). Architecture: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

---

## 1. Apple signing (in progress, blocked on account activation)

The whole point: an unsigned app makes Gatekeeper block the download, and makes
macOS forget the keychain grant on every rebuild. Both stop the day this lands.
`build.yml` already reads all six secrets and degrades to unsigned without them.

- [x] Owner: Apple Developer account purchased 2026-07-31. Waiting on activation.
- [x] Me: private key and CSR generated at `~/Documents/inkwell-signing/`.
- [ ] Owner: create a **Developer ID Application** certificate from that CSR at developer.apple.com, download the `.cer` into the same folder. Needs the Account Holder role.
- [ ] Owner: app-specific password at appleid.apple.com, set with `gh secret set APPLE_PASSWORD -R SirSicard/inkwell` so it is typed by the owner and never passes through anyone else.
- [ ] Owner: paste the Team ID and the developer-account Apple ID (neither is secret; both are embedded in every signed build).
- [ ] Me: convert `.cer` plus key to `.p12`, set `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_TEAM_ID`.
- [ ] Me: cut 0.2.2 and verify with `spctl --assess` and `stapler validate` on the downloaded dmg, not by trusting a green CI run.
- [ ] Me: once notarized, delete the quarantine workaround from the Homebrew cask and the `xattr -cr` instructions from the README. They exist only because the app is unsigned.
- [ ] Owner: back up `~/Documents/inkwell-signing/developer-id.key`. Apple can reissue a certificate; that private key cannot be recovered.

## 2. Windows

Building is solved (sherpa-onnx compiles on MSVC, v0.2.1 ships msi and exe).
Nothing about *using* it has ever been checked.

- [ ] Owner: a Win11 test environment. Free route (UTM plus CrystalFetch) written up in [docs/owner-tasks.md](docs/owner-tasks.md). Unactivated Windows is legal and sufficient; the VM emulates x64 on Apple Silicon, so it proves behaviour and says nothing about speed.
- [ ] Me/owner: QA pass. Hotkey, paste target, overlay placement, keyring (windows-native backend), per-app detection (process-name path, never exercised), voice editing's copy keystroke.
- [ ] Owner: Windows code signing, least urgent of the four. Routes and the eligibility check first in [docs/owner-tasks.md](docs/owner-tasks.md). With current download numbers this buys little; reasonable to defer until someone reports being blocked.

## 3. Accuracy, round two

Round one shipped in 0.2.1: hotword biasing from the dictionary, trim-only VAD,
quiet-point chunking, pre-roll and release-tail capture, transient-proof
normalisation. The tooling to judge round two exists and is unused.

- [ ] Owner: record a corpus. Step by step in [docs/owner-tasks.md](docs/owner-tasks.md), including which five kinds of sentence to cover and why each one tests a different stage. Note the toggle is in **General** in v0.2.1, not Troubleshooting: that tab exists only in later builds.
- [ ] Me: run `cargo run --release --example ab_models` over that corpus to rank Parakeet V3 int8, V2 fp16 and V3 full precision on the owner's own voice. Until then, model choice is a guess with numbers attached to the wrong thing.
- [ ] Me: Qwen3-ASR-0.6B needs sherpa-onnx 1.12 to 1.13, which swaps the static libraries under everything. Do it alone so a broken upgrade cannot hold anything else hostage. It is the only candidate with a published accented-English win.
- [ ] Me: decide `advanced_mode`'s future. It gates which tabs appear, which is navigation, not dictation; it may belong in the sidebar rather than in General.

## 4. Streaming partials (designed, not built)

Spike verdict: viable at 40x real time with no dependency upgrade, but streaming
output has no casing or punctuation and drops the last word, so it can never be
the text the user keeps.

- [ ] Me: streaming model feeds the overlay while the key is held. Never pasted, never stored. Offline pass unchanged.
- [ ] Me: render partials lowercase, or the switch to properly-cased final text reads as a glitch rather than as refinement.
- [ ] Me: opt-in setting defaulting to off, described as "show words as you speak" rather than as a second model. Costs a 296 MB download.

## 5. Distribution

- [ ] Owner: buy a domain. Availability checked 2026-07-31 and candidates listed in [docs/owner-tasks.md](docs/owner-tasks.md); every short option is gone, `inkwell.tools` and two-word `.com`s are the realistic tier.
- [ ] Owner, deferred on purpose: a `homebrew-tap` repo. The cask is written and passes `brew audit --strict`, so this is two minutes whenever it is wanted, but the value is currently thin and worth stating rather than assuming. The official homebrew-cask repo needs notability the project does not have (0 stars, 0 forks, 0 watchers as of 2026-07-31), so a personal tap is the only route, and `brew install --cask sirsicard/tap/inkwell` is a longer instruction than downloading the dmg. Homebrew's other draw, `brew upgrade`, is already covered by the app's own updater. Against that it adds a permanent chore: the sha256 must be bumped every release or installs fail on a checksum mismatch. Publish it when somebody asks for brew, or when traction allows submitting to homebrew-cask proper and earning the short name.
- [ ] Me, after the domain: move the updater off `workers.dev` (route already stubbed in `inkwell-updater/wrangler.toml`), then re-verify a version hop.
- [ ] Me: per-tab screenshots. The dashboard shot is in the README; the sidebar is a webview and synthetic clicks would not switch tabs, so the rest are worth doing by hand.
- [ ] Me: demo GIF. Best recorded against a throwaway profile, like the screenshot, so no real transcript is ever published.

## Standing rules learned the hard way

- After every release run `inkwell-updater/publish-latest.sh`. The worker serves from KV, not from GitHub, so a release nobody pushes to KV updates nobody.
- After every homepage deploy, `vercel alias set <deployment-url> getinkwell.vercel.app`. Vercel does not move the alias itself.
- Never screenshot the owner's own profile. Use a throwaway HOME with symlinked models; the real dashboard shows real transcripts and publishing is permanent.

## Done, for the record

v0.2.0 and v0.2.1 released and public. Repo public as `SirSicard/inkwell`.
Homepage live. Updater verified end to end. Voice editing shipped. Clean-machine
first run verified with zero panics and every catalogue URL live. Homebrew cask
written and audited. Debug audio, Troubleshooting tab, voice-command mode
pinning, README screenshot.

## Not doing

Meeting mode, speaker diarization, calendar integration, agent mode, chainable
transform chains, portable mode, GPU and CUDA plumbing, mobile, browser
extension, telemetry, any paid tier.
