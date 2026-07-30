# Inkwell - Roadmap

*Ordered by dependency, not by date. No dates are promised.*
*State audited 2026-07-30. The rehaul itself (sections 0 to 3 of the old file) is done and lives in the git log and CHANGELOG; this file only tracks what is left.*

Analysis behind the rehaul: [docs/rehaul-analysis-2026-07-24.md](docs/rehaul-analysis-2026-07-24.md). Feature research: [docs/competitive-extras-2026-07-27.md](docs/competitive-extras-2026-07-27.md). Architecture: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

---

## 1. Ship v0.2.0 (critical path, in order)

The repo has zero releases. Every download link 404s until this section is done.

- [ ] Owner: set the `TAURI_SIGNING_PRIVATE_KEY` repo secret from `src-tauri/.tauri-private.key`. CI preflight hard-fails without it, by design.
- [ ] Owner: back the minisign private key up outside GitHub secrets (password manager). Losing it permanently breaks the update chain for every installed copy.
- [ ] Local `tauri build` dry run to prove the bundler produces a `.dmg` before the first tag burns a CI run.
- [ ] Bump `0.1.1` to `0.2.0` in `src-tauri/tauri.conf.json` + `Cargo.toml`, fix `package.json` (still `0.0.0`) and `APP_VERSION` in `homepage/lib/constants.ts`. Write release notes from the commit range.
- [ ] Tag `v0.2.0`. First-ever run of `build.yml`; macOS arm64 and Windows are `best_effort: false`.
- [ ] Owner: make the repo public. Renamed to `SirSicard/inkwell` 2026-07-30; still private.
- [ ] Owner: pick the real domain, set `SITE_URL` (still `inkwell.example`).
- [ ] Deploy the homepage (local-only today, no Vercel link).

## 2. Signing (the two recurring costs)

- [ ] Owner: Apple Developer account ($99/yr). Unblocks Developer ID + notarization (already wired in `build.yml`, degrades to unsigned), and is the real fix for the keychain "Always Allow" never sticking to dev builds.
- [ ] Add the Apple secrets to CI and verify a signed, notarized build.
- [ ] Windows signing: Azure Trusted Signing (~$10/mo) if individual eligibility checks out, else an OV cert. Until then SmartScreen warns on every download.

## 3. Windows platform pass

- [ ] `workflow_dispatch` test build first: sherpa-onnx 1.12 static has never been compiled on MSVC. Same risk class as the Apple Silicon linking was. Do this before spending anything on Windows.
- [ ] Win11 test environment. Unactivated Win11 in a VM is legal and enough for dev testing. Parallels on the Mac runs Windows ARM and emulates x64: fine for "does the hotkey work", meaningless for inference benchmarks.
- [ ] Windows QA: hotkey, paste target, overlay, keyring (windows-native), per-app detection (process-name path).

## 4. Verification before calling it launched

- [ ] End-to-end updater test with a real version hop (0.2.0 to 0.2.1-pre). The worker answers 204 today; a full update has never happened.
- [ ] Clean-machine first-run: onboarding, model download, mic + accessibility permission flow.
- [ ] Owner: daily-driver pilot of the new features (modes, cleanup, pause, teach-from-transcript, polish with the keychain granted).
- [ ] Owner: confirm in the Cloudflare dashboard that the old free-tier worker (`inkwell-worker`) is actually deleted, not just its source. It 404s today, which suggests gone; wrangler is not authenticated locally so it is unconfirmed.

## 5. Features, in the order the research ranks them

- [ ] Voice editing / Command Mode: select text, hold a second hotkey, speak an instruction, get the rewrite pasted over the selection. The last converged competitor gap. Was gated on BYOK polish working, which it now does.
- [ ] Streaming spike: streaming Zipformer vs Moonshine v2 vs an Apple SpeechAnalyzer sidecar. Parakeet is offline-only in sherpa-onnx, so live partials need a second model either way. Research: [docs/research/raw/08-streaming.md](docs/research/raw/08-streaming.md).
- [ ] If the spike survives: two-pass pipeline, streaming partials in the overlay, final pass rescored offline.

## 6. Distribution polish

- [ ] Move the updater endpoint off `workers.dev` onto the owned domain (route is already stubbed in `inkwell-updater/wrangler.toml`), then re-verify an update on both platforms.
- [ ] Homebrew cask.
- [ ] Screenshot and demo GIF in the README.

## 7. Housekeeping (low urgency)

- [ ] Fold `debug_save_audio` into a Troubleshooting section instead of General.
- [ ] Decide whether voice commands fold into Modes or stay a separate concept.
- [ ] Intel macOS and Linux builds stay `best_effort: true`; promote only if users show up.

## Not doing

Meeting mode, speaker diarization, calendar integration, agent mode, chainable transform chains, portable mode, GPU and CUDA plumbing, mobile, browser extension, telemetry, any paid tier.
