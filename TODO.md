# Inkwell - Roadmap

*Ordered by dependency, not by date. No dates are promised.*
*State audited 2026-08-26. v0.2.8 is the current release, all four platforms green, both dmgs notarised and verified, the updater serving it. Signing, notarisation and the release chain are finished problems. What is left is reach, dependency debt, and the parts of the app nobody has ever run.*

Analysis behind the rehaul: [docs/rehaul-analysis-2026-07-24.md](docs/rehaul-analysis-2026-07-24.md). Feature research: [docs/competitive-extras-2026-07-27.md](docs/competitive-extras-2026-07-27.md). Streaming verdict: [docs/streaming-spike-2026-07-31.md](docs/streaming-spike-2026-07-31.md). Architecture: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). Release steps: [docs/RELEASING.md](docs/RELEASING.md).

---

## 0. The uncomfortable number

44 downloads, 1 star, 0 forks, 0 issues, across five releases. The engineering
is further along than the distribution, and an hour spent on reach is currently
worth more than an hour spent on the list below. Kept at the top on purpose,
because everything under it is easier and none of it is more important.

- [ ] Owner/me: a custom social preview image. `usesCustomOpenGraphImage` is
      false, so every share of the repo on Slack, Discord or X renders as a
      generic card. Half an hour, and it is the cheapest thing here.
- [ ] Me: demo GIF. Hold, speak, release, text appears is a six-second loop that
      sells itself, and the README currently describes it in prose. Record it
      against a throwaway profile, never a real transcript history.
- [ ] Owner: somewhere to post it. The product is genuinely ready for an
      audience in a way it was not two weeks ago.

## 1. Dependency debt (compounding)

Thirteen open Dependabot PRs, all green, none merged. The config groups minor
and patch, so **every open one is a major**. This only gets harder.

- [ ] Me: merge the build-time ones together (typescript 5.9 to 7.0, eslint 9 to
      10, `@types/node`, `@eslint/js`). No runtime risk.
- [ ] Me: then one at a time, each with a real dictation afterwards, because CI
      passing proves nothing about any of them:
      **enigo 0.3 to 0.6** (owns the synthetic paste, three majors on the
      library whose permission behaviour took two releases to get right),
      **cpal 0.17 to 0.18** (owns capture, including the idle-release stream
      lifecycle), **rubato 0.16 to 4.0** (resampling; a silent behaviour change
      here degrades every transcript), **symphonia 0.5 to 0.6** (file
      transcription only).

## 2. The parts nobody has run

- [ ] Owner: a Win11 test environment. Free route (UTM plus CrystalFetch) in
      [docs/owner-tasks.md](docs/owner-tasks.md). The VM emulates x64 on Apple
      Silicon, so it proves behaviour and says nothing about speed.
- [ ] Me/owner: Windows QA. Hotkey, paste target, overlay placement, keyring
      (windows-native backend), per-app detection (process-name path, never
      exercised), voice editing's copy keystroke. A Windows build ships every
      release and nobody has ever launched one.
- [ ] Windows SmartScreen. **Code signing no longer fixes this**: Microsoft's own
      docs now say a valid OV or EV certificate still shows "unrecognized app"
      until reputation accumulates, and EV lost its bypass in 2024. The only
      path that removes the warning is the **Microsoft Store**, which is now free
      for individuals, signs the app for Microsoft, and needs an MSIX. Gated on
      Windows QA first, and on testing that MSIX packaging does not break the
      global hotkey or the synthetic paste.
- [ ] Linux: best effort, still unverified. Fine to leave.

## 3. Tests where the breakage actually is

118 tests, and they cluster in pure functions. The stateful code has none.

- [ ] Me: `pipeline.rs` (917 lines, 0 tests). `on_hotkey` now takes
      `(handle, is_edit, pressed)`, so the start/stop state machine is finally
      testable without a running app. Highest value test in the repo.
- [ ] Me: `paste.rs` (348 lines, 0 tests). Clipboard save and restore, and the
      no-prompt permission paths.
- [ ] Me: a frontend test runner. There is none at all; roughly 4,000 lines of
      TypeScript have never been asserted on.

## 4. Streaming partials (designed, not built)

Spike verdict: viable at 40x real time with no dependency upgrade, but streaming
output has no casing or punctuation and drops the last word, so it can never be
the text the user keeps. Still the best-argued unbuilt feature here: it fills
the silent gap between releasing the key and seeing the paste.

- [ ] Me: streaming model feeds the overlay while the key is held. Never pasted, never stored. Offline pass unchanged.
- [ ] Me: render partials lowercase, or the switch to properly-cased final text reads as a glitch rather than as refinement.
- [ ] Me: opt-in setting defaulting to off, described as "show words as you speak" rather than as a second model. Costs a 296 MB download.

## 5. Distribution and loose ends

- [ ] **Owner: back up `~/Documents/inkwell-signing/developer-id.key`.** Apple can
      reissue a certificate; that private key cannot be recovered. Oldest open
      item here and the only one that is unrecoverable if ignored.
- [ ] Owner: buy a domain. Candidates in [docs/owner-tasks.md](docs/owner-tasks.md); `inkwell.tools` was free as of 2026-07-31.
- [ ] Me, after the domain: move the updater off `workers.dev` (route stubbed in `inkwell-updater/wrangler.toml`), then re-verify a version hop.
- [ ] Owner, deferred on purpose: a `homebrew-tap` repo. The cask is written and
      audited, and `bin/update-cask.sh` now removes the per-release chore that
      was the main argument against it. The remaining argument stands:
      homebrew-cask proper needs notability the project does not have, a
      personal tap means a longer install line than downloading the dmg, and
      `brew upgrade` duplicates the app's own updater. Publish it when somebody
      asks.
- [ ] Me: per-tab screenshots. Three exist (dashboard, modes, AI). The rest are
      cheap now that `src/devmock.ts` renders the real frontend in a browser
      against fixtures.
- [ ] Me: decide `advanced_mode`'s future. It gates which tabs appear, which is
      navigation, not dictation; it may belong in the sidebar rather than in General.

## Standing rules learned the hard way

- **Verify the artefact, not the build.** A green CI run said the app was
  notarised while the dmg around it was not; that shipped as 0.2.5 and was
  caught by asking Gatekeeper, not by reading a log.
- **After every release run `inkwell-updater/publish-latest.sh`.** The worker
  serves from KV, not from GitHub. It has failed twice; it now retries and reads
  the value back, but check the version it prints.
- **Never screenshot the owner's own profile.** Use a throwaway HOME with
  symlinked models, or `src/devmock.ts` in a browser. The real dashboard shows
  real transcripts and publishing is permanent.
- The homepage alias rule is **gone**: `getinkwell.vercel.app` is a project
  domain bound to Production and follows deployments by itself. Do not reinstate
  `vercel alias set`.

## Done, for the record

Signed with a Developer ID and notarised by Apple, dmg included, verified under
quarantine on both architectures. Updater verified end to end across version
hops. Five models measured on a real corpus and the catalogue cut from thirteen
to five. Voice editing. Modes. Qwen3 as the accuracy tier. Release logging that
does not write your dictations to disk. A Grant permission button that can
recover a lost Accessibility grant. The microphone released when idle, so the
machine sleeps and auto-locks again. A watchdog that unsticks a recording whose
key-release event never arrived. A space between consecutive dictations. A Stats
page. Single-key and modifier-only (Fn, right Command) hotkeys. Homepage
auto-deploying from Git. Security alerts at zero.

## Not doing

Meeting mode, speaker diarization, calendar integration, agent mode, chainable
transform chains, portable mode, GPU and CUDA plumbing, mobile, browser
extension, telemetry, any paid tier.
