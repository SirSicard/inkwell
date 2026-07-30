# Inkwell - Roadmap

*Ordered by dependency, not by date. No dates are promised.*
*State audited 2026-07-30, evening. v0.2.0 shipped that day: repo public as SirSicard/inkwell, release live with signed artifacts for Apple Silicon, Windows x64 and Linux x64, homepage deployed to Vercel, donation link real. The rehaul history lives in CHANGELOG.md and the git log.*

Analysis behind the rehaul: [docs/rehaul-analysis-2026-07-24.md](docs/rehaul-analysis-2026-07-24.md). Feature research: [docs/competitive-extras-2026-07-27.md](docs/competitive-extras-2026-07-27.md). Architecture: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

---

## 1. Launch tail (small, high leverage)

- [ ] Owner: disable Vercel deployment protection so the site is public. vercel.com > project inkwell > Settings > Deployment Protection > Vercel Authentication > Disabled. Until then https://getinkwell.vercel.app bounces every visitor to a Vercel login.
- [ ] Owner: `npx wrangler login` in `inkwell-updater/` (browser OAuth, cannot be done for you).
- [ ] Then me: push the 0.2.0 manifest to the `INKWELL_RELEASES` KV key and verify the endpoint serves it. The worker reads KV, not GitHub, so until this push no installed copy is ever offered 0.2.0.
- [ ] Then me: confirm the old free-tier worker (`inkwell-worker`) is deleted at Cloudflare, not just 404ing.
- [ ] Owner, optional: buy a real domain. getinkwell.vercel.app works and is name-neutral; an owned domain upgrades the homepage, the updater endpoint and the OG links in one move.

## 2. Signing (the two recurring costs)

- [ ] Owner: Apple Developer account ($99/yr). Unblocks Developer ID + notarization (already wired in `build.yml`, degrades to unsigned) and permanently ends the keychain re-prompt loop that dev builds cause.
- [ ] Me: add the Apple secrets to CI, cut a signed 0.2.1, verify notarization.
- [ ] Owner: Windows signing, Azure Trusted Signing (~$10/mo) if individual eligibility checks out. Until then SmartScreen warns on every download.

## 3. Windows QA

The build question is answered: sherpa-onnx compiles on MSVC and the v0.2.0 msi/exe exist. What remains is using it.

- [ ] Owner: a Win11 test environment. Unactivated Win11 in a VM is legal and enough. Parallels on the Mac runs Windows ARM and emulates x64: fine for behaviour, meaningless for inference benchmarks.
- [ ] Me/owner: QA pass, including hotkey, paste target, overlay, keyring (windows-native), per-app detection (process-name path).

## 4. Verification before calling it launched

- [ ] End-to-end updater test with a real version hop, after the KV push.
- [ ] Clean-machine first-run: fresh macOS account, install the released dmg, walk onboarding (model download, mic + accessibility permissions).
- [ ] Owner: keep daily-driving. AI polish confirmed working end to end 2026-07-30 (three consecutive dictations polished after Allow was granted).
- [ ] Next tag: confirm `macos-15-intel` actually supplies runners. macos-13 was retired, which is why v0.2.0 has no Intel build; the new label is unproven.

## 5. Features, in the order the research ranks them

- [ ] Voice editing / Command Mode: select text, hold a second hotkey, speak an instruction, get the rewrite pasted over the selection. The last converged competitor gap. Nothing blocks it.
- [ ] Streaming spike: streaming Zipformer vs Moonshine v2 vs an Apple SpeechAnalyzer sidecar. Parakeet is offline-only in sherpa-onnx, so live partials need a second model either way. Research: [docs/research/raw/08-streaming.md](docs/research/raw/08-streaming.md).
- [ ] If the spike survives: two-pass pipeline, streaming partials in the overlay, final pass rescored offline.

## 6. Distribution polish

- [ ] Move the updater endpoint off `workers.dev` onto the owned domain (route stubbed in `inkwell-updater/wrangler.toml`), then re-verify.
- [ ] Homebrew cask (possible now that the release is public; nicer after signing).
- [ ] Screenshot and demo GIF in the README.

## 7. Housekeeping (low urgency)

- [ ] Fold `debug_save_audio` into a Troubleshooting section instead of General.
- [ ] Decide whether voice commands fold into Modes or stay a separate concept.
- [ ] Homepage deploys must re-point the alias afterwards: `vercel alias set <deployment-url> getinkwell.vercel.app`. Automate or document in CI if deploys become frequent.

## Not doing

Meeting mode, speaker diarization, calendar integration, agent mode, chainable transform chains, portable mode, GPU and CUDA plumbing, mobile, browser extension, telemetry, any paid tier.
