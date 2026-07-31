# Inkwell - Roadmap

*Ordered by dependency, not by date. No dates are promised.*
*State audited 2026-07-30, evening. v0.2.0 shipped that day: repo public as SirSicard/inkwell, release live with signed artifacts for Apple Silicon, Windows x64 and Linux x64, homepage deployed to Vercel, donation link real. The rehaul history lives in CHANGELOG.md and the git log.*

Analysis behind the rehaul: [docs/rehaul-analysis-2026-07-24.md](docs/rehaul-analysis-2026-07-24.md). Feature research: [docs/competitive-extras-2026-07-27.md](docs/competitive-extras-2026-07-27.md). Architecture: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

---

## 1. Launch tail: DONE 2026-07-30 evening

- [x] Homepage public at https://getinkwell.vercel.app (deployment protection off, verified serving with the real donate link).
- [x] Updater live end to end: 0.2.0 manifest pushed to KV, a 0.1.1 client is offered 0.2.0 with a valid signature, a 0.2.0 client gets 204. The worker itself was redeployed (the running copy was from March and predated the pre-release comparison fix).
- [x] Old free-tier worker confirmed deleted at Cloudflare (API error 10007, does not exist), not just 404ing.
- [x] After every future release run `inkwell-updater/publish-latest.sh`: the worker reads KV, not GitHub, so a release nobody pushes to KV updates nobody.
- [ ] Owner, someday: buy a real domain. getinkwell.vercel.app works and is name-neutral; an owned domain upgrades the homepage, the updater endpoint and the OG links in one move.

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

## 5. Transcription accuracy (opened after the 2026-07-30 deep dive)

The diagnosis, from the owner's own 29-transcript history (docs of record: the
transcription-quality workflow results): proper nouns failed from vocabulary,
not acoustics ("Inkwell" 0-for-5); every duplicated-word artifact came from 15s
chunk seams; short clips lost word edges to push-to-talk timing. Mic and core
model were explicitly exonerated. Fixed in-tree the same day: hotword biasing
from the dictionary + beam search, trim-only VAD, quiet-point 60s chunking,
transient-proof normalization, pre-roll/release-tail capture, resampler flush.

- [x] Owner added 13 dictionary entries 2026-07-30; they now bias the decoder itself, not just post-editing.
- [x] A/B harness: `cargo run --release --example ab_models` scores every installed model over a corpus by word error rate, biased with the app's own dictionary. Validated on synthetic speech.
- [x] Debug audio writes one file per take to ~/Documents/Inkwell Debug Audio. It used to overwrite a single file in the system temp dir, so a corpus could never accumulate, which is why the first attempt at collecting one produced nothing.
- [x] Candidates #2 and #3 are in the model list and installable: Parakeet V2 fp16 (1.3 GB) and Parakeet V3 full precision (2.5 GB, external weights file).
- [ ] Candidate #1, Qwen3-ASR-0.6B int8: needs the sherpa-onnx crate to go 1.12 -> 1.13 (new static libs, unproven API). Deliberately not bundled into the 0.2.1 release; do it on its own so a broken upgrade cannot hold accuracy fixes hostage.
- [x] filetranscribe cuts at the quietest point near each 30s boundary instead of a blind offset.
- [x] transcripts.audio_duration_ms now records the length of the user's actual press, not of whatever reached the recognizer, so internal padding changes stop silently rewriting stored stats.
- [ ] Owner: record a corpus (Save Debug Audio on, dictate, write take-NNNN.txt next to each wav), then run the harness to decide between the three Parakeet builds on your own voice.

## 6. Features, in the order the research ranks them

- [ ] Voice editing / Command Mode: select text, hold a second hotkey, speak an instruction, get the rewrite pasted over the selection. The last converged competitor gap. Nothing blocks it.
- [ ] Streaming spike: streaming Zipformer vs Moonshine v2 vs an Apple SpeechAnalyzer sidecar. Parakeet is offline-only in sherpa-onnx, so live partials need a second model either way. Research: [docs/research/raw/08-streaming.md](docs/research/raw/08-streaming.md).
- [ ] If the spike survives: two-pass pipeline, streaming partials in the overlay, final pass rescored offline.

## 7. Distribution polish

- [ ] Move the updater endpoint off `workers.dev` onto the owned domain (route stubbed in `inkwell-updater/wrangler.toml`), then re-verify.
- [ ] Homebrew cask (possible now that the release is public; nicer after signing).
- [ ] Screenshot and demo GIF in the README.

## 8. Housekeeping (low urgency)

- [x] `debug_save_audio` moved to its own Troubleshooting tab with a button that reveals the recordings folder. It is a diagnostic you switch on for an hour, not a preference you set once.
- [x] Voice commands stay a separate concept: a command is an action, a mode is a way of writing, and folding them would make "mode" mean two things. But the style commands were fixed, because they had been dead since modes took ownership of style: they wrote a field the pipeline no longer reads. They now pin a mode, the pin beats app matching, and the Modes tab shows it with a Clear button so it is never invisible state.
- [ ] Homepage deploys must re-point the alias afterwards: `vercel alias set <deployment-url> getinkwell.vercel.app`. Automate or document in CI if deploys become frequent.

## Not doing

Meeting mode, speaker diarization, calendar integration, agent mode, chainable transform chains, portable mode, GPU and CUDA plumbing, mobile, browser extension, telemetry, any paid tier.
