# Mac integration handoff — 2026-10-08

## Waiting on the owner

The owner authorized quitting the running RC and the two pinned source downloads. Available live scratch checks and the macOS26-targeted rebuild are complete; the existing RC was restarted. Remaining environment checks: runtime on an actual macOS26 machine, speech/transcription/insertion with real models, physical global shortcut and window-focus-loss behavior, and the live recording pill. No models were downloaded or owner model/library data used. UI automation routes key events to the app and cannot prove the hardware event tap. Minimum-width live dragging was unavailable through the UI server; existing720pt layout checks remain passing.

## Integrated source

- Integration branch: `integ/mac-1.0`, advanced by local fast-forward from `codex/mac-windows-integration`. Both worktrees were clean after integration.
- Verified base: `0ea5bd252c48c7796ae82647101b07e1fefc1b10`.
- Imported bundle tip: `df38fc42a84d7053480965375aaf1be39b988bc3`.
- Imported tree: `c563abf1358902dfdaf4b062404a1066a0502203`.
- Bundle SHA256: `4d5b63a908aaea4e4c994ea1d8bcfe98c504252427e7b4307354212cdf913d66`.
- Bundle verification/base availability and fetched tip/tree matched. All 21 commits integrated by fast-forward; no merge conflicts. Benchmark branch excluded.
- Mac parity commit: `469030e1`; bundler repair commit: `9df95d5f07bc96ccbcb1cb270522591c40594984`.
- RC code tree: `e09bef3fd3cd7df7113f3e4e3e63a99e58a6135f`. A subsequent documentation commit does not change the built product.

## Mac parity

- Remembered-app removal: confirmation describes inherited default, rediscovery and retained past recordings; changed default reconfirms; unreadable store refuses; correlated persisted acknowledgement removes the row.
- Meeting shortcut: separate Mac event tap, default Off, persisted native setting/state, independent toggle. Recorder requires both correlated suspension acknowledgements, waits for key/modifier release before successful rearm, resumes on cancellation/failure/timeouts, clears on core stop and preserves three-way conflicts and stored keys. Voice edit uses the existing recorder beside AI.
- Mode Save: approval is independent of the global polish switch and bound to the exact editor, model, destination, provider model and name override. Revalidation before Allow and after reply, exact confirmed pin, revoke behavior and stopped-core editor recovery preserved.
- Imported shared prompt/live-offline changes compile on Mac. Qwen local context is 8192; Foundation Models retains its independently reported context/default4096. FluidAudio was not replaced or retuned; Swift engine API unchanged.
- Orb strength10–100 by tens/default70, renderer uniform boost, shared fading and accessibility caps. Recording pill has left orb, larger centered app, separate white REC/coral dot, palette wash and visible fallback. Consent/warning layouts and nonactivation preserved; action contrast corrected in both modes.
- Native Settings phrase hierarchy, persistent form labels, responsive model status and expandable accuracy details retain existing source figures.
- Canonical events/shaders verified current without unnecessary regeneration. SwiftPM's exact verified build-local framework rpath is removed before existing strict linkage/signing checks; unknown absolute paths and symlink escapes remain refused.

## Verification actually run

Checks used installed caches, temporary data and the two subsequently authorized pinned dependency source downloads. No model/tool/dataset downloads, owner library/credentials/recordings/transcripts used for testing, real-model benchmarks, release installation or publishing. An initial app-name lookup automatically returned the legacy dashboard; its contents were not used or copied. Subsequent UI checks bound the exact scratch executable.

- Rust workspace gate: **1908 passed,20 ignored**,123 test executables covered. The initial run found one key-validation regression; resumed remaining binaries found three outdated conflict expectations. Each was corrected and its exact repro passed; completed unchanged binaries were not repeated. Workspace doc checks exit0.
- Rust fmt and all-targets workspace clippy: exit0. Engine-enabled llama/Silero/NeMo adapters: clippy exit0; **201 passed,38 ignored** in that configuration. Engine-enabled privacy logger regression: **1/1**.
- Swift gate: **724 passed,13 skipped**,737 final production test cases covered across the interrupted run, remaining cases and focused corrections. Initial Settings identity crash fixed and its layout repro/card checks passed3/3. Old blot positive-control threshold reconciled with intended shader radius/softness: background edge20/full93 in both themes, strict background limit retained. Exact mode/recorder regressions49/49. Four failures in the original Rust attempt and the interrupted Swift failure are preserved in raw evidence, not described as initial green runs.
- Swift skips: four AMI/live real-model cases; three Foundation Models opt-in cases; four optional icon/Drop/Stats export cases; GPU timing opt-in; onscreen frame-rate case (window not on screen). No TSan run: changed Swift models remain MainActor-isolated; targeted lifecycle/acknowledgement regressions ran. Swiftlint/swift-format are unavailable and were not installed.
- Mac script gate:17 existing scripts/509 assertions passed; new real-Mach-O rpath regression8 assertions passed. Rust licence/bans/sources and Swift pins/checkouts/binaries licence audits pass; Rust notices current.
- Canonical native debug and release engine builds succeed. The subsequent macOS26 engine release build passes strict linkage with no newer-macOS allowance:95 Mach-O files,89 loose engine libraries and one framework; every file targets26.0 or older. Signature/deep strict verification, hardened runtime/audio-input/calendar entitlements and designated requirement matching the existing RC pass. Debug-only replay/cloud hooks absent from release as checked by bundler.
- macOS26 dependency rebuild: the canonical script verified both authorized archive hashes;96 Abseil/SentencePiece and89 NeMo prefix dylibs independently checked for arm64/minimum26.0, signatures and manifest hashes. NeMo/ggml local pinned sources copied to scratch; existing prefixes/source unchanged. Rebuilt-library regressions:7 passed,10 ignored, plus the explicit installed-library ggml isolation case1/1. Full unchanged suites were not repeated.
- Specialist Swift/Rust/security briefs applied once to risky changes; findings resolved and checked. No unresolved critical/high review findings.

Raw checks remain locally under `integration-evidence/` (excluded from git). Input Windows evidence remains in the extracted scratch handoff.

## Visual evidence and limits

- Whole Settings offscreen Light/Dark at720/1040: `integration-evidence/settings-renders/`. Card bounds and window width assertions pass.
- Pill native Metal composition/fallback, manual/automatic/offer/long app names in both modes, plus narrow Appearance/Models/Voice crops: scratch `renders/` accompanying the RC manifest. Recording status, warning text/actions and corrected button contrast inspected.
- Offscreen AppKit images exclude some blur/Metal layers; native orb images were composited with the actual installed Metal shader. Additional live scratch Light/Dark Appearance screenshots and Dark voice-command hierarchy are in the chat tool record. Presets realized correctly; orb80 and the recorded meeting shortcut persisted after restart. Escape restored enabled shortcut controls; recording a conflicting dictation chord preserved both keys and showed the canonical refusal. Voice-edit consent cancellation left edit Off and resumed the controls. Remembered-app removal showed inheritance/rediscovery/past-recording wording and removed the row after acknowledgement. Microphone test detected32 percent, stopped, and reported “Inkwell heard you.” A short meeting start/stop returned idle with an explicit no-audio warning; this is not a successful transcription check. Installed badge/expanded accuracy still need installed-model live verification. No retained scratch audio or transcript was opened/exported. Existing layout constraint warnings occurred while bounds assertions passed.
- The original RC remains preserved with its macOS27-only dependency limitation. The new RC removes that binary-minimum limitation through pinned-source rebuilds and passes strict26.0 linkage. It starts on the current macOS27 host; runtime on macOS26 itself is still untested. No privacy, linkage or signing refusal was bypassed.

## Local RC artifact

- New archive: `Inkwell_1.0.0_mac-integration-macOS26-local-RC.zip` (scratch output only, not installed); ZIP integrity passes.
- SHA256: `a9bc93c78926ffe80b3e05d3a0beb706e1cdc0ca61464e900719cffa53f7545b`.
- Source commit `6d755bf499c5a51ee296a31112e7ea572d011e34`; source/options in adjacent `RC-MANIFEST-macOS26.json`; no signing identity recorded. Documentation-only follow-up does not change the product.
- Earlier macOS27-only archive and its `RC-MANIFEST.json` retained separately.
- Signed with stable local identity, no timestamp/notarization/DMG/public release.

## Windows identity and deferred scope

Supplied Windows handoff, not rerun on Mac: installer `Inkwell_1.0.0_x64-setup.exe`, source `df38fc42a84d7053480965375aaf1be39b988bc3`, SHA256 `9875eadd6443e9db5504141484ee17aa53c9ac85c31b86bb894b9f6a88ec0c0e`. Mac integration does not change that installer. Windows .NET/WinUI and manual runtime checks were not verified here. Windows checks1/2 are waived, not passed. WhatsApp remains deferred and excluded. Spanish benchmark wording, narrow Windows greeting and inactive undo/pause remain supplied nonblocking notes.

No push, tag, PR, publishing, deployment or release installation. Original working changes preserved.
