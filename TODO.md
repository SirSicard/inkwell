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

- [x] A custom social preview image. Live: the repo's `og:image` now resolves to
      `repository-images.githubusercontent.com`, not the generated fallback.
      Source and png are in `docs/media/social-preview.*`; the png is committed
      because GitHub has no API for this and the upload is a manual step under
      Settings > General.
- [ ] **Owner-blocked: demo GIF.** Hold, speak, release, text appears is a
      six-second loop that sells itself, and the README describes it in prose.
      This one needs a person: it is a real screen recording of a real voice
      pasting into a real app, and the two things that would let it be made
      unattended are both disqualifying. Compositing the overlay over a fake
      editor produces a mockup presented as a recording, and driving the real
      app needs the microphone, the Accessibility grant and the keyboard focus
      of whoever is sitting there. Five minutes of owner time, on a throwaway
      profile, never a real transcript history.
- [ ] Owner: somewhere to post it. The product is genuinely ready for an
      audience in a way it was not two weeks ago.

## 1. Dependency debt (compounding)

The config groups minor and patch, so **every open Dependabot PR is a major**.
Worked through in `df6184c`; what is left is left for a reason.

- [x] **enigo 0.3 to 0.6.** Owns the synthetic paste. Taken because
      `open_prompt_to_get_permissions` still exists in 0.6.1 and still means what
      `paste.rs` relies on it meaning; that flag is the reason Inkwell does not
      raise a system prompt mid-dictation, and a rename would have been silent.
- [x] rusqlite 0.33 to 0.40, `@types/node` 24 to 26, homepage and updater minors.
- [ ] **Blocked upstream, not deferred: typescript 7 and eslint 10.**
      `typescript-eslint` pins `typescript >=4.8.4 <6.1.0`, and its latest
      release still resolves `@eslint/js@9`. Nothing to do until it ships.
      Re-check when `typescript-eslint` cuts a major.
- [ ] **API breaks, not bumps: cpal 0.17 to 0.18, rubato 0.16 to 4.0,
      symphonia 0.5 to 0.6.** Each was attempted and reverted (6, 1 and 1
      compile errors respectively). These are migrations that need a real
      dictation afterwards, not upgrades: cpal owns capture including the
      idle-release stream lifecycle, and a silent resampling change in rubato
      degrades every transcript without failing anything.

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

195 tests, and they cluster in pure functions. The stateful code has almost none.

- [x] `pipeline.rs`: the start/stop state machine. `decide_transition(pressed,
      is_recording, mode, is_edit)` is now a pure function with 7 tests, so the
      push-to-talk and toggle paths are asserted without a running app. It is
      what both the OS hotkey plugin and the Fn event tap route through.
- [ ] Me: the rest of `pipeline.rs`. `stop_and_process` and `watchdog_loop` still
      need a handle, so they stay untested until the app state they touch is
      behind something injectable.
- [ ] Me: `paste.rs` (348 lines, 0 tests). Clipboard save and restore, and the
      no-prompt permission paths.
- [ ] Me: a frontend test runner. There is none at all; roughly 4,000 lines of
      TypeScript have never been asserted on.

## 4. Live preview (built, unvalidated by a human)

Shipped as "Live Preview" in General, off by default. Everything up to the
moment real microphone audio arrives is covered by
`examples/streaming_check.rs`, which drives the shipped config rather than a
copy of it: partials at 43x real time, and a resampler within 0.001% of real
time at 16k, 22.05k, 44.1k and 48k with an identical transcript at all four.

- [x] Streaming model feeds the overlay while the key is held. Never pasted,
      never stored, offline pass untouched.
- [x] Partials lowercase, so the switch to properly-cased final text reads as
      refinement rather than correction.
- [x] Opt-in, defaulting to off. **73 MB, not 296.** That figure was the whole
      HuggingFace directory, which carries both precisions and two
      left-context variants; the four files needed are a tenth of it. It was
      most of the argument against building this.
- [x] The overlay grows from 97px to 560px to hold the words, and is the same
      box it always was when the feature is off. The spike said "feed the
      overlay" without noticing the overlay is a 97px square.
- [ ] **Owner: does it feel right?** The one thing no harness can answer. Turn
      it on, dictate a long sentence, and watch whether the words landing
      helps or distracts. Known and expected: the first word or two are weak
      until the decoder warms up, and the last word never appears, which is
      the spike's tail-loss finding and is what the offline pass fixes.
- [ ] Me, if it survives that: partials for voice edits are currently on the
      same path, which has had no thought put into it beyond "the overlay is
      showing anyway".

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
auto-deploying from Git. Security alerts at zero. A social preview image, so a
shared link stops rendering as a generic card. Live preview: words on the
overlay while you are still speaking.

## Not doing

Meeting mode, speaker diarization, calendar integration, agent mode, chainable
transform chains, portable mode, GPU and CUDA plumbing, mobile, browser
extension, telemetry, any paid tier.
