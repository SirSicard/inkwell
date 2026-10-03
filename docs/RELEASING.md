# Releasing

Two release chains share this repository until the 0.2 app retires:

- **Inkwell 1.x, the native Mac app** (`mac/`): `.github/workflows/mac-release.yml`, on `v1.X.Y`
  tags. It is described first.
- **Inkwell 1.x on Windows** (`windows/`): `.github/workflows/win-release.yml`, on the same tags,
  into the same release. It is described [after the Mac's](#inkwell-1x-on-windows).
- **Inkwell 0.2, the Tauri app** (branch `legacy/0.2`): `build.yml`, on `v0.*` tags. Its chain is
  [below](#inkwell-02-the-tauri-app).

## Inkwell 1.x, the Mac app

On a `v1.X.Y` tag the workflow builds the app, signs it with the Developer ID, notarizes and staples
the app and then the dmg, asks Gatekeeper about both the way a user's Mac will, signs the Sparkle
appcast, and **drafts** a GitHub release holding the dmg, `appcast.xml` and the build manifest.
Nothing is public until a person publishes the draft. Installed apps read the feed of the latest
published release:

    https://github.com/SirSicard/inkwell/releases/latest/download/appcast.xml

| Job | Runs on | Holds | Does |
|---|---|---|---|
| `build` | tag and dry run | the `APPLE_*` secrets; read access | build the engines, then the app; sign, notarize, staple, Gatekeeper check, the build manifest; the dmg and the manifest as run artifacts |
| `appcast-rehearsal` | dry run only | nothing secret; read access | the appcast signed with a throwaway key, and checked |
| `publish` | tag only | `SPARKLE_ED_PRIVATE_KEY` (environment `release`); write access | the appcast signed and checked against the app's key; the draft release with the dmg, the appcast and the build manifest |

Everything a step does lives in a script under `mac/scripts/` that runs the same on a Mac:
`build-core.sh`, `build-mac.sh --timestamp --engines`, `notarize.sh`, `package-dmg.sh`,
`verify-release.sh`, `build-manifest.sh`, `sparkle-tools.sh` and `appcast.sh` (with
`appcast-check.swift`), and
`core/crates/ink-engines/native/build-nemo-speech.sh`. Each prints nothing that names the signing
identity (`lib/redact-signing.sh`).

### The engines

A release carries its speech engines: Qwen3-ASR on llama.cpp (the dictation and meeting finals),
Silero VAD on tract, and Nemotron diarization of the far end on NeMo-Speech.cpp. Their model
weights are never in the app: it downloads them when they are first needed
([MODEL-WEIGHTS.md](MODEL-WEIGHTS.md)). `build-mac.sh --timestamp` refuses to build without
`--engines`, and `--engines` refuses a core built with any other feature set.

- **What ships where.** llama.cpp (with its ggml) and tract are linked into the core statically.
  NeMo-Speech.cpp is a set of shared libraries with its own ggml, built from its pinned commit by
  `build-nemo-speech.sh`, and goes in `Contents/Frameworks` with the SentencePiece and Abseil
  libraries it loads, each signed like the app
  ([ARCHITECTURE.md](ARCHITECTURE.md), "The diarizer's native library").
- **SentencePiece and Abseil, pinned.** `core/crates/ink-engines/native/build-sentencepiece-abseil.sh`
  fetches SentencePiece 0.2.2 and Abseil 20260817.0 as their release tarballs, refuses either unless
  its SHA-256 is the one committed in the script, and builds both for arm64 and macOS 26 with no
  host-specific flags, SentencePiece against that Abseil, every library installed as `@rpath/<name>`
  with `@loader_path` as its only rpath. The workflow builds them on every run (no cache) and hands
  the prefix to `build-nemo-speech.sh` as `ENGINE_DEPS_DIR`, which builds NeMo-Speech.cpp against
  them, copies them into its own prefix, and stops if any copy came from anywhere else. Homebrew
  serves CMake and Ninja only. `build-mac.sh` fails if any Mach-O in the app loads a path outside the
  bundle and the OS, has an absolute rpath, or loads a library it cannot find inside the bundle.
  To move to a new version: change the version, URL and SHA-256 in the script (read the hash from
  the release, never retype it), check the licences again, and run a dry run.
- **The build manifest.** `Inkwell_X.Y.Z_build-manifest.txt` is an asset of every release, beside
  the dmg (a dry run keeps it as the artifact `inkwell-build-manifest`), so what a shipped dmg
  contains outlives the run's log: the source commit and the dmg's SHA-256; NeMo-Speech.cpp's
  commit, its `GGML_NATIVE` and each prefix library's SHA-256; SentencePiece's and Abseil's pinned
  versions and tarball SHA-256s; each bundled library, by project, version and file; and
  `brew list --versions` of the build tools. `build-manifest.sh` writes no path of the build
  machine: it refuses a library that was not copied from the pinned prefix, one that changed after
  that prefix was built, and a prefix built from any tarball but the pinned ones.
- **Every Apple silicon Mac on macOS 26.** Both ggml copies are built with `GGML_NATIVE=OFF` and no
  `-march`, the M1's instruction set: `build-core.sh` reads llama.cpp's value back from its CMake
  cache, and ink-engines' `build.rs` refuses a NeMo prefix whose manifest does not record it.
  Everything is built for macOS 26, and `build-mac.sh` fails on any Mach-O built for a newer one.
  The pinned SentencePiece and Abseil are built for macOS 26 wherever they are built. A local build
  that uses Homebrew's instead (no `ENGINE_DEPS_DIR`) on a Mac running a newer macOS gets libraries
  built for that one, and needs `INK_ALLOW_NEWER_MACOS=1` (a warning instead of a failure;
  `--timestamp` refuses it); that build will not start on macOS 26.
- **The diarizer's source.** The workflow fetches NeMo-Speech.cpp at the commit
  `build-nemo-speech.sh` pins, with its ggml submodule; the script checks both commits.

The same build on a Mac (for a local check; the release is CI's):

```bash
brew install cmake ninja
core/crates/ink-engines/native/build-sentencepiece-abseil.sh ~/engine-deps ~/engine-deps-work
commit="$(sed -n 's/^NEMO_COMMIT=//p' core/crates/ink-engines/native/build-nemo-speech.sh)"
git clone https://github.com/NVIDIA/NeMo-Speech.cpp ~/src/NeMo-Speech.cpp
git -C ~/src/NeMo-Speech.cpp checkout "$commit" && git -C ~/src/NeMo-Speech.cpp submodule update --init ggml
ENGINE_DEPS_DIR=~/engine-deps core/crates/ink-engines/native/build-nemo-speech.sh ~/src/NeMo-Speech.cpp ~/nemo-speech
NEMO_SPEECH_DIR=~/nemo-speech mac/scripts/build-mac.sh --engines
```

### The licence notices

Settings > About carries the notice of everything the app ships that is not Inkwell's own. Two
lists, kept in two ways:

- **By hand:** `mac/Sources/Inkwell/Screens/Notices.swift`, for the C and C++ code, the Swift
  packages, the engines' libraries and the model weights. Each needs a row in
  [THIRD_PARTY.md](../THIRD_PARTY.md), and `NoticesTests` fails on a row without its notice.
- **Generated:** `mac/Sources/Inkwell/Generated/RustNotices.swift`, the licence files of every
  third-party Rust crate the release links into the core, from cargo's resolution of the release
  build (`cargo run -p ink-ffi --bin ink-notices`, offline). The core's tests and the Swift tests
  fail while it was generated from another `core/Cargo.lock`, and `mac/scripts/rust-notices.sh
  --check` (the Mac CI job's step, and release day's) fails on any difference from a fresh run.
  After a dependency change, `mac/scripts/rust-notices.sh` regenerates it (it fetches the crates
  first). A crate whose package carries no text of its licence stops the generator until
  `core/crates/ink-ffi/notices/overrides.txt` supplies one for that version.
- **Checked against upstream before a release.** Each text written without its upstream file on
  hand (a line of `overrides.txt`, and the notices `Notices.swift` composes, listed in
  `mac/composed-notices.txt`) carries `verified=no` until someone compares it with that file, and
  the date after. `mac/scripts/notices-verified.sh` lists the open ones; `release-version.sh`
  refuses a release tag while there is one, and a dry run only lists them.

### Once: the update key (the maintainer, by hand)

Sparkle refuses any update whose archive is not signed with the EdDSA key whose public half the
installed app carries. The key is the maintainer's alone: no script makes it, and no agent handles
it. Until it exists, `SUPublicEDKey` in `mac/Info.plist` is empty, the app starts no updater, and a
tag's run stops at its first check (the dry run still runs, and says so).

1. **Sparkle's tools**, checked against the pinned hash (the 2.10.0 release, as `mac/Package.swift`):

   ```bash
   mac/scripts/sparkle-tools.sh ~/sparkle-2.10.0
   ```

2. **Make the key.** It goes into the login keychain (account `ed25519`), and the tool prints the
   public half:

   ```bash
   ~/sparkle-2.10.0/bin/generate_keys
   ```

   Lost, no installed copy can ever be updated again: people would have to download the next
   release by hand. Leaked, anyone who can also publish a release here could ship an update.
   (Rotating it takes a release signed with the old key that carries the new public key.) Step 4
   exports it once; that file, stored encrypted and offline, is the backup.

3. **The environment.** In the repository's Settings > Environments, create `release`. Under
   Deployment branches and tags choose "Selected branches and tags" and add the **tag** rule
   `v1.*`: only a job running on such a tag can then read the environment's secrets. A required
   reviewer (yourself) is optional: `publish` would then wait for an approval. From a shell
   (untested; the web page does the same):

   ```bash
   gh api -X PUT repos/SirSicard/inkwell/environments/release \
     -F 'deployment_branch_policy[protected_branches]=false' \
     -F 'deployment_branch_policy[custom_branch_policies]=true'
   gh api -X POST repos/SirSicard/inkwell/environments/release/deployment-branch-policies \
     -f name='v1.*' -f type=tag
   ```

4. **The secret `SPARKLE_ED_PRIVATE_KEY`**, in the `release` environment, not the repository:

   ```bash
   ~/sparkle-2.10.0/bin/generate_keys -x ~/sparkle-private-key
   gh secret set SPARKLE_ED_PRIVATE_KEY --env release --repo SirSicard/inkwell < ~/sparkle-private-key
   # Now move the file into your encrypted backup, and leave no copy here.
   ```

5. **The public half into the app.** `~/sparkle-2.10.0/bin/generate_keys -p` prints it. Put it in
   `mac/Info.plist` as the `SUPublicEDKey` string, with nothing around it, and merge that through a
   pull request (`ShippedUpdateSettingsTests` refuses a key that is not 32 bytes of base64).

6. **Recommended:** a tag ruleset (Settings > Rules) that lets only you create or move `v1.*` tags.
   The workflow already refuses a tag on a commit that `main` does not contain.

The signing and notarization secrets are the ones the 0.2 chain uses: `APPLE_CERTIFICATE` (the
Developer ID Application certificate and key, a base64 .p12), `APPLE_CERTIFICATE_PASSWORD`,
`APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD` (an app-specific password) and
`APPLE_TEAM_ID`.

### Step 0: the dry run

A manual run builds, signs and notarizes exactly as a tag does, keeps the notarized dmg and the
build manifest as run artifacts for 14 days, rehearses the appcast with a throwaway key, and
publishes nothing. Run it before touching a version number; a failure here costs a re-run, the
same failure after tagging a deleted tag.

```bash
gh workflow run mac-release.yml --repo SirSicard/inkwell --ref main -f version=1.0.0
run="$(gh run list --repo SirSicard/inkwell --workflow mac-release.yml --limit 1 --json databaseId --jq '.[0].databaseId')"
gh run watch "$run" --repo SirSicard/inkwell
gh run download "$run" --repo SirSicard/inkwell --name inkwell-dmg --dir ~/Downloads/inkwell-dry-run
gh run download "$run" --repo SirSicard/inkwell --name inkwell-build-manifest --dir ~/Downloads/inkwell-dry-run
mac/scripts/verify-release.sh ~/Downloads/inkwell-dry-run/Inkwell_1.0.0_aarch64.dmg --notarized --version 1.0.0
spctl -a -vv -t open --context context:primary-signature ~/Downloads/inkwell-dry-run/Inkwell_1.0.0_aarch64.dmg
```

`verify-release.sh` quarantines a copy first, as a browser does, and must end with both verdicts
`accepted`, `source=Notarized Developer ID`. A dispatch needs the workflow on `main` first.

### Cut it

```bash
# 0. One pull request, merged to main: the CHANGELOG heading (## [Unreleased] -> ## [X.Y.Z] - date)
#    and the core's version. The app's version comes from the tag (build-mac.sh writes it into the
#    bundle, the Windows build into the app); the core's is core/Cargo.toml's [workspace.package]
#    version, which About shows as "core X.Y.Z" on both. Set it to X.Y.Z, then the lock and the
#    Rust notices, which record the lock's fingerprint:
(cd core && cargo update --workspace)
mac/scripts/rust-notices.sh
(cd core && cargo run -p ink-ffi --bin ink-notices -- --windows)
# 1. On an up-to-date main: the Rust notices match its Cargo.lock and a fresh run, so the release
#    ships what About lists (fetches the crates it has not got; fails naming any crate whose
#    notice is missing or stale), and the core says the release's version. Both tags' first step
#    runs the second check too (core-version.sh), and stops there when it fails.
mac/scripts/rust-notices.sh --check
mac/scripts/core-version.sh X.Y.Z
# 2. Tag main and push the tag. Only v1.X.Y exactly (no suffix: it is also CFBundleVersion, which
#    Sparkle compares); a pre-release is a dry run.
git fetch origin && git tag -a v1.X.Y -m "Inkwell X.Y.Z" origin/main && git push origin v1.X.Y
```

### After CI goes green

```bash
# 3. The draft's dmg, checked the way a user's Mac will check it.
gh release download v1.X.Y --repo SirSicard/inkwell --pattern '*.dmg' --dir ~/Downloads/inkwell-vX.Y.Z
mac/scripts/verify-release.sh ~/Downloads/inkwell-vX.Y.Z/Inkwell_X.Y.Z_aarch64.dmg --notarized --version X.Y.Z --update-key

# 4. Publish. This is the moment installed apps can see it: the feed URL follows "latest".
gh release edit v1.X.Y --repo SirSicard/inkwell --draft=false --latest

# 5. The feed now names the new version.
curl -sL https://github.com/SirSicard/inkwell/releases/latest/download/appcast.xml | grep -m1 '<sparkle:version>'
```

**Publish in order, and only 1.x as latest.** The feed is the latest published release's
`appcast.xml`, and each release's appcast is built from the one published before it. So publish
drafts in version order, and publish any later 0.2.x release with `--latest=false`: a 0.2 release
marked latest has no appcast, and every installed 1.x app would stop finding updates.

### What guards the update chain

- **Two signatures on every update.** Sparkle installs an update only if its archive's EdDSA
  signature verifies against the installed app's `SUPublicEDKey` (checked before it unpacks the
  archive: `SUVerifyUpdateBeforeExtraction`) and the new app is signed by the same Developer ID team.
  The feed itself is signed too (`SURequireSignedFeed`).
- **Checked before it ships.** `appcast.sh` reads the key out of the app in the dmg and checks the
  new feed and item against it, so a feed signed with any other key fails in CI rather than on
  users' Macs. The previous feed is checked the same way before its items are carried over.
- **Secrets by job.** The Apple secrets never share a job with write access; the EdDSA key is read
  only in `publish`, only on a `v1.*` tag, and goes to Sparkle's tools on stdin, never to disk.
  Neither trigger can come from a pull request, so no fork's code runs next to a secret.
- **Pinned inputs.** Actions by commit; Sparkle's framework by URL and SHA-256 in `mac/Package.swift`
  (and in the Swift licence audit's vetted list); Sparkle's tools by size and SHA-256 in
  `sparkle-tools.sh`; llama.cpp by the exact `llama-cpp-2` version in `Cargo.lock`; NeMo-Speech.cpp
  and its ggml by commit; SentencePiece 0.2.2 and Abseil 20260817.0 by tarball SHA-256 in
  `build-sentencepiece-abseil.sh` (recorded in the release's build manifest). Not pinned: CMake and
  Ninja, Homebrew's current builds (recorded too). The release build restores no cache.
- **Third-party build code runs before the key is there.** The engines (NeMo-Speech.cpp's CMake
  build, the core's cargo build scripts) are built before the signing keychain is created, so no
  build script ever runs next to an unlocked Developer ID key.
- **No new entitlements.** The app is not sandboxed, so Sparkle installs through its `Autoupdate`
  tool and `Updater.app` as ordinary helpers and never starts its XPC services (they serve sandboxed
  apps, which opt in through Info.plist). `build-mac.sh` signs all of Sparkle's code with the app's
  identity, hardened runtime and timestamp, and fails if any signature but the app's carries an
  entitlement. The engines' libraries in `Contents/Frameworks` are signed the same way. The app
  keeps only `audio-input`.
- **Asked, not assumed.** Sparkle asks the user before its first automatic check, and a check sends
  no system profile (`SUEnableSystemProfiling` off).

### Release day: 1.0.0 (once)

What the first 1.x release needs beyond the chain above, in order. The 0.2 app stays on `main`
until this day and is not touched before it (invariant I6 in [ARCHITECTURE.md](ARCHITECTURE.md)).

1.0 is one release on the Mac and on Windows together: the tag waits until the Windows app is
ready too. Windows ships unsigned at first, with the homepage explaining how to install it past
SmartScreen; its release chain is [below](#inkwell-1x-on-windows). Linux and Intel Macs are
dropped: 0.2 is their last version.

Before the tag:

- [x] **The maintainer:** the update key, the `release` environment and its secret, the public
      key in `mac/Info.plist`, and the tag ruleset (the steps under "Once: the update key").
- [ ] Every notice written without its upstream file compared with that project's own licence
      file, replaced where it differs, and its `verified=` set to the date: the lines of
      `core/crates/ink-ffi/notices/overrides.txt` (then `mac/scripts/rust-notices.sh`), of
      `mac/composed-notices.txt` (replacing the text in `Notices.swift`), and of the Windows-only
      `windows/Inkwell.Core/Screens/About/composed-notices.txt` (replacing the text in
      `Notices.cs`). `mac/scripts/notices-verified.sh` and
      `windows/scripts/release-version.sh tag v1.0.0` (the same check with the Windows list) must
      pass: each platform's tag refuses to build until its check does.
- [ ] The 0.2 app removed from `main` in its own pull request (`legacy/0.2` keeps it): `src/`,
      `src-tauri/`, `public/`, `index.html`, `package.json`, `package-lock.json`,
      `vite.config.ts`, `eslint.config.js` and the three `tsconfig*.json`. With them, what points
      at them: `.github/dependabot.yml`'s npm entry for `/` and cargo entry for `/src-tauri`;
      `.gitignore`'s `src-tauri` lines; `build.yml` (a `v0.*` tag builds from `legacy/0.2`'s own
      copy); `CLAUDE.md` and `CONTRIBUTING.md`; invariant I6 in `ARCHITECTURE.md`, which ends here;
      the rows of `THIRD_PARTY.md` that point into `src/` or `src-tauri/`, and the matching
      exception in `NoticesTests`; the scripts only 0.2 uses (`scripts/download-models.*`,
      `scripts/gen-model-chart.py`); and `TODO.md`, the 0.2 work list. `docs/legacy/` stays.
- [x] The core at 1.0.0 (`core/Cargo.toml`), so About says "core 1.0.0" on both platforms;
      `mac/scripts/core-version.sh 1.0.0` passes.
- [ ] The README rewritten for 1.0.
- [ ] The Windows app ready for the same release, through its own chain.
- [ ] Step 0, the dry run, on the commit to be tagged; then "Cut it" with `v1.0.0`.

After CI goes green, on the draft, before anything is published:

- [ ] Step 3 above: the draft's dmg checked with `--update-key`.
- [ ] **The maintainer:** that dmg installed on a fresh macOS user account (onboarding, one
      dictation, one meeting), and on the everyday account over the installed 0.2: it replaces
      it in place, and the microphone and Accessibility grants carry over (same bundle id and
      team; compare `codesign -d -r-` of both apps).
- [ ] **The maintainer:** the draft's Windows installer (`gh release download v1.0.0 --repo
      SirSicard/inkwell --pattern '*_x64-setup.exe'`) on a fresh Windows account (one dictation,
      one meeting), and on an account with 0.2 installed (`windows/S3.6-CHECKLIST.md`, section 6).

Then:

- [ ] Steps 4 and 5 above: published as latest, the feed read back.
- [ ] 0.2.10 from `legacy/0.2` with an in-app notice pointing to 1.0 (0.2's updater cannot
      install 1.0): the 0.2 chain below, published with `--latest=false` so that 1.0 stays the
      release the feed follows, then `inkwell-updater/publish-latest.sh` and `bin/update-cask.sh`
      (the cask moves to 0.2.10, so Homebrew users get the notice too). Skip the 0.2 chain's step
      8: the homepage shows 1.0. On Linux and Intel Macs the notice says 0.2 is their last version.
- [ ] `inkwell-updater/` retired once 1.0 has shipped on Windows too and 0.2.10 has gone out
      through it (0.2 installs read the notice from it until then).
- No Homebrew cask for 1.0: `packaging/homebrew/inkwell.rb` stays on 0.2 (0.2.10 once the notice
  release is out). 1.x gets its cask at 1.0.1 (macOS 26 or later; 1.0's data folder,
  `~/Library/Application Support/Inkwell`, in `zap`; then `bin/update-cask.sh`).
- [ ] The homepage's `APP_VERSION` and release snapshot, only once 1.0.0 is published (step 8 of
      the 0.2 chain), in a commit authored as SirSicard: Vercel builds no other author's commits.

## Inkwell 1.x on Windows

The same `v1.X.Y` tag starts `win-release.yml` beside `mac-release.yml`. On x64, the one Windows
architecture 1.0 ships (`win-release-build.yml`, on `windows-2025`; ARM64 waits for 1.0.1), it
builds the core with the Windows engines and the app ReadyToRun, checks what they need from a
PC, packs the installer and the update feed with Velopack, and then adds them to the tag's
**draft** release (creating it if the Mac's workflow has not yet). A manual run is the dry run:
everything but the release, the files kept as the run artifact `inkwell-windows` for 14 days.

The engines it ships (`windows/scripts/build-core.ps1`, whose list ink-notices' Windows features
are held to): Qwen3-ASR on llama.cpp for dictation and meeting finals (Vulkan, and the CPU where a
PC has no Vulkan GPU), Silero VAD on tract, Parakeet v3 int8 on sherpa-onnx (live words; dictation
without a GPU), and Nemotron diarization of the far end on NeMo-Speech.cpp (Vulkan, then the CPU).

- **sherpa-onnx**: its 1.13.4 "shared, MD, Release, no-tts" archive for x64, fetched
  by `win-release-build.yml` and checked against the SHA-256 GitHub publishes for it, and every file
  the build uses from it against ink-engines' `build.rs` pins (`SHERPA_ONNX_DIR`). No
  text-to-speech, so none of espeak-ng. Its `sherpa-onnx-c-api.dll`, `onnxruntime.dll` and
  `onnxruntime_providers_shared.dll` ship beside `Inkwell.exe`.
- **NeMo-Speech.cpp**: built from its pinned commit with `build-nemo-speech.sh` (Git Bash inside
  the developer environment), against SentencePiece and Abseil built from their pinned tarballs by
  `build-sentencepiece-abseil.sh` and linked into NeMo's own DLL (`ENGINE_DEPS_DIR`), as for the
  Mac. Its `nemo_speech_asr_c.dll`, `nemo_speech_asr.dll` and its ggml's DLLs ship beside
  `Inkwell.exe`. The core delay-loads it: its Vulkan backend needs the Vulkan loader as soon as it
  loads, so a PC without a Vulkan driver runs everything but the diarizer.
- **The Visual C++ runtime**: both engines' DLLs link it dynamically (`/MD`), so `build-core.ps1`
  copies the DLLs of it they import, and no others, from the build machine's Visual Studio 2026
  redistributable folder (`VCToolsRedistDir`: `Microsoft.VC145.CRT` and `Microsoft.VC145.OpenMP`;
  on the runner and on a PC today `vcruntime140.dll`, `vcruntime140_1.dll`, `msvcp140.dll`,
  `msvcp140_1.dll` and `vcomp140.dll`). They ship beside `Inkwell.exe` (app-local, so the per-user
  install needs no administrator and no Visual C++ Redistributable). Another redistributable than
  VC145 stops the build: About shows the licence terms of this one. The build prints the version,
  and the release notes give it (`win-release-build.yml`'s `vcruntime` output).
- `build-core.ps1` puts the core's DLL, the engines' DLLs and that runtime in
  `core/target/release/inkwell-core/`, and the publish takes every DLL there (`InkCoreDir`).

**Unsigned, for now.** The Windows build is not code-signed. SmartScreen warns before the installer
runs, and Smart App Control blocks it outright; the homepage says how to check the download and get
past SmartScreen (`windows/HOMEPAGE-INSTALL.md` is its draft). Signing is a later step.

| Job | Runs on | Holds | Does |
|---|---|---|---|
| `build` (`win-release-build.yml`) | tag and dry run | nothing secret; read access | sherpa-onnx's archive, the Vulkan SDK (pinned by LunarG's published SHA-256) and the diarizer's prefix, all pinned; `windows/scripts/build-core.ps1`; the generated-code and notice checks; the locked restore and NuGet licence check; the ReadyToRun publish with the tag's version; `windows/scripts/pack.ps1` |
| `publish` | tag only | write access (environment `release`) | the files checked against their SHA-256s, then added to the tag's draft release, with a Windows section in its notes |

What a release carries for Windows:

- `Inkwell_X.Y.Z_x64-setup.exe`, the installer. Per user, no administrator: it installs into
  `%LOCALAPPDATA%\InkwellApp`, adds a Start menu entry and an entry in Settings > Apps, and starts
  the app. Uninstalling removes that folder only: the library, in `%LOCALAPPDATA%\Inkwell`, stays.
  It refuses Windows older than 11 24H2. The package id `InkwellApp` is the update chain's name: it
  never changes. While it installs it shows a splash with Microsoft's end-user terms, which the
  licences of the Windows App SDK (its section 3(b)(ii)), of the Windows SDK's .NET projection and
  of Visual Studio for the Visual C++ runtime (their Distribution Requirements) ask for, drawn by
  `pack.ps1`; the release notes and the homepage carry them too, and Settings > About in full.
  Velopack's installer has no licence page, so the app's first run asks instead: the terms and the
  three licences, with Agree and Quit, before anything else starts (`TermsStep`). The agreement is
  kept in `terms-agreed.txt` in the library folder with the terms' version, a SHA-256 of the
  sentence and the three licence texts (`TermsStep.CurrentVersion`): editing the sentence or
  re-copying any of the licences asks every existing user again at their next start.
- `InkwellApp-X.Y.Z-full.nupkg` and `releases.win.json`: the update and its feed (Velopack's
  channel `win`, which is part of the update chain too: it never changes). The app's Settings >
  About > Check Now reads the feeds of the repository's latest published releases (GitHub's API,
  then the assets over HTTPS), and Velopack installs a package only if its size and SHA-256 match
  the feed's. `pack.ps1` checks the feed against the package it wrote.
- `Inkwell_X.Y.Z_windows-sha256.txt`: the SHA-256s of the three, in `sha256sum` format. The
  installer's is also in the release notes.

What the checks guarantee:

- **The Visual C++ runtime is the app's own.** The core's DLL links the CRT statically
  (`crt-static`, and llama.cpp's CMake build with `CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded`); the
  .NET and Windows App SDK binaries use only the UCRT, which is part of Windows; the engines' DLLs
  need the Visual C++ runtime DLLs above, and `build-core.ps1` (for the core and the engines' DLLs)
  and `pack.ps1` (for the whole app) accept one only when it is beside `Inkwell.exe` and built for
  x64, never the build machine's copy in System32 (nor the debug UCRT). `pack.ps1` prints the
  runtime's version and refuses DLLs of two versions.
- **Everything it loads is in the package or part of Windows.** Every DLL a binary of the app
  loads when it loads must be beside `Inkwell.exe` or part of Windows (`windows/scripts/lib/dll-imports.ps1`,
  which both scripts use). A file in the build machine's System32 counts only if Windows signs it
  as its own, so nothing a PC gets elsewhere passes for Windows because the build machine has it:
  the Vulkan loader (GPU drivers), a Visual C++ runtime, LLVM's OpenMP. ONNX Runtime never
  counts: Windows 11 has an older `onnxruntime.dll` of its own there, and sherpa-onnx must get the
  one the app ships (the adapter also refuses any other version when it loads).
- **It starts without Vulkan.** `vulkan-1.dll` is delay-loaded, and only for the functions
  `build-core.ps1` lists; the core then runs llama.cpp on the CPU (`tests/vulkan_missing.rs` runs
  in the same job). The diarizer's DLL is delay-loaded too, and its Vulkan backend is the one
  binary allowed to need the loader when it loads; the core checks the diarizer loads before its
  first call (ink-engines' `src/nemo.rs`).
- **It is x64's.** `pack.ps1` refuses an `Inkwell.exe` or core built for another architecture.
- **The core's version.** A tag waits, as the Mac's does, until `core/Cargo.toml` says the tag's
  version, which About shows as the core's (`windows/scripts/release-version.sh` runs
  `mac/scripts/core-version.sh`, as the Mac's release-version.sh does); a dry run says so and
  goes on.
- **Notices first.** A tag waits, as the Mac's does, for every notice written without its upstream
  file to be compared with it, the Windows-only ones (`windows/Inkwell.Core/Screens/About/composed-notices.txt`)
  included (`windows/scripts/release-version.sh`).

**When a PC reports a problem.** The app keeps a log of its own on the PC and never sends it
anywhere: `logs\inkwell.log` in the library folder (`%LOCALAPPDATA%\Inkwell\logs`, or under
`INK_DATA_DIR`), up to 1 MB, with the two before it as `inkwell.1.log` and `inkwell.2.log`
(`windows/Inkwell.Core/LocalLog.cs`). It holds the shell's diagnostics (`ScreenLog`: what failed,
by command name and fixed words, never a command's fields) and what the core writes to stderr at
its default level, info. The core keeps what was said out of its lines; they can name files in
the library and models folders, and quote an online provider's error. Anything else in the
process that writes to stderr is marked `stderr`. When an exception ends the app, it first writes
`crash-YYYYMMDD-HHMMSS.txt` beside the log: the time, the app's and Windows' versions, and each
exception's type and stack, without its message (the newest ten are kept). A crash inside native
code (the core, an engine, a GPU driver) ends the process without a note. Ask the user for the
files; uninstalling leaves them, with the library.

The same build on a PC (Visual Studio's C++ build tools, CMake, Ninja, LLVM, Git Bash, the Vulkan
SDK with `VULKAN_SDK` set, the .NET SDK `windows/global.json` pins), x64:

```powershell
# SHERPA_ONNX_DIR: the unpacked sherpa-onnx-v1.13.4-win-x64-shared-MD-Release-no-tts-lib archive.
# NEMO_SPEECH_DIR: the prefix build-nemo-speech.sh installed (ENGINE_DEPS_DIR from
# build-sentencepiece-abseil.sh), both run from Git Bash inside Visual Studio's developer
# environment, as win-release-build.yml runs them.
pwsh windows/scripts/build-core.ps1
cd windows
dotnet restore Inkwell.slnx --locked-mode
dotnet publish Inkwell/Inkwell.csproj -c Release -o $env:TEMP\inkwell-app --no-restore -p:InkVersion=1.0.0 "-p:InkCoreDir=$PWD\..\core\target\release\inkwell-core\"
pwsh scripts/pack.ps1 -Version 1.0.0 -AppDir $env:TEMP\inkwell-app -OutDir $env:TEMP\inkwell-release
```

**Publishing.** The draft holds both platforms' files once both workflows are green. Read the notes
before publishing: whichever workflow drafted the release wrote its opening, and the Mac's workflow
does not add its paragraph to a draft the Windows workflow made. Then step 4 of the Mac's chain
publishes both at once. Every 1.x release carries both platforms: the Windows app looks for its
feed in the latest ten releases, so a Mac-only release does not stop Windows updates, but a
Windows user would miss a fix that only shipped on the Mac.

**The dry run** (a dispatch needs the workflow on `main` first):

```bash
gh workflow run win-release.yml --repo SirSicard/inkwell --ref main -f version=1.0.0
run="$(gh run list --repo SirSicard/inkwell --workflow win-release.yml --limit 1 --json databaseId --jq '.[0].databaseId')"
gh run watch "$run" --repo SirSicard/inkwell
gh run download "$run" --repo SirSicard/inkwell --name inkwell-windows --dir ~/Downloads/inkwell-windows-dry-run
```

Then `windows/S3.6-CHECKLIST.md` on a PC with the downloaded installer.

## Inkwell 0.2, the Tauri app

This chain runs from the `legacy/0.2` branch, where `build.yml` builds on `v0.*` tags.

The chain used to be six manual steps held in one person's head, and two of
them failed silently in production: the updater manifest push exited without
writing (0.2.6, unnoticed for two days) and the canonical URL stayed pinned to
the previous build (0.2.5 and 0.2.6). Both now either automate themselves or
refuse to lie about having worked.

### First, prove it builds

**Nothing compiles this repository on push.** `build.yml` triggers on `v*` tags
and on manual dispatch only, so between one release and the next, Windows and
Linux are never built and `cargo test` never runs anywhere but a laptop. The
first cross-platform compile of a month's work would otherwise be the release
itself, with the tag already pushed.

A manual run is the dry run for this. Every publishing step in the workflow is
gated on `refs/tags/`, `tagName` resolves to an empty string off a tag, and
there is a dedicated step that uploads the bundles as artifacts instead. So this
builds all four platforms, notarises the macOS dmgs, and publishes nothing:

```bash
gh workflow run build.yml --ref main
gh run watch "$(gh run list --workflow build.yml --limit 1 --json databaseId --jq '.[0].databaseId')"
```

Do this before touching a version number. A failure here costs a re-push; the
same failure after tagging costs a deleted tag and a burnt version.

### Cut it

```bash
# 1. Bump the version in FOUR places, plus Cargo.lock
#    src-tauri/Cargo.toml, src-tauri/tauri.conf.json, package.json,
#    and the CHANGELOG heading (## [Unreleased] -> ## [X.Y.Z] - date)
#    (cd src-tauri && cargo check)   regenerates Cargo.lock
#
#    NOT homepage/src/lib/constants.ts. That is the fifth place and it waits
#    for step 7, because the homepage deploys on push and its own rule is
#    that the site may only advertise a version a release exists for.
#    Bumping it here puts the new number on a page whose Download button
#    still hands out the old build for as long as CI takes.
#
#    macOS note: BSD sed has no `0,/re/` address form. It fails silently,
#    leaving the version untouched, which is easy to miss and then tag.

# 2. Parity check. This is the one that only fails under the Tauri CLI:
#    a Rust/npm Tauri version mismatch compiles fine and dies in CI.
npm run tauri info | sed -n '/Packages/,/^$/p'

# 3. Tag and push. CI builds four platforms, notarises both dmgs, and
#    self-verifies with stapler validate plus spctl before uploading.
git tag -a vX.Y.Z -m "Inkwell X.Y.Z" && git push origin vX.Y.Z
```

### After CI goes green

```bash
# 4. Verify the artefact the way a user's Mac will: fresh download, browser
#    quarantine flag, then ask Gatekeeper. A green notarisation submission is
#    not a passing verdict; that exact gap shipped as 0.2.5.
#    (see the verify-dmg snippet in this file's history, or run by hand:)
xattr -w com.apple.quarantine "0081;$(printf %x $(date +%s));Safari;" Inkwell_X.Y.Z_aarch64.dmg
spctl --assess -vv --type open --context context:primary-signature Inkwell_X.Y.Z_aarch64.dmg

# 5. Publish
gh release edit vX.Y.Z --draft=false --latest

# 6. Push the updater manifest into Cloudflare KV. Retries once and then reads
#    the value back, so it cannot report success without having written.
inkwell-updater/publish-latest.sh

# 7. Point the cask at the release. Refuses on a draft, on a no-op rewrite,
#    and on a URL that does not return 200.
bin/update-cask.sh

# 8. Now set APP_VERSION in homepage/src/lib/constants.ts to the same
#    version, run (cd homepage && node scripts/snapshot-release.mjs) to copy
#    the release's asset list into src/data/release.json, and push. The
#    release exists, so the site can describe it honestly, and the build
#    refuses a version the snapshot doesn't match. Pushing this is what
#    deploys the homepage.
```

### What no longer needs doing

**The homepage.** The `inkwell` Vercel project is connected to this repository
with root directory `homepage`, so pushing to `main` deploys it. "Include files
outside the root directory" is off and "Skip deployments" is on, which is what
stops a Rust-only commit from rebuilding the site: the homepage is
self-contained (own lockfile, no imports above its own directory), so nothing
outside it can affect the build.

**Moving the alias.** `getinkwell.vercel.app` is a project domain bound to the
Production environment, so it follows the newest production deployment on its
own. It used to be a hand-pinned alias, which is why `vercel --prod` moved the
three auto-generated domains and left the canonical URL a release behind.
