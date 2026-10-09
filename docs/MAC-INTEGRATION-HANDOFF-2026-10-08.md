# Mac integration handoff — 2026-10-08

## Waiting on the owner

Local integration and available native checks are complete. Remaining checks require an actual macOS26 host and physical keyboard/spoken-input interaction: this Mac runs27.2, and routed UI key events do not reach the global hardware event tap. Minimum-width live dragging did not resize the window; existing720pt layout checks pass. No models were downloaded: public-fixture replay used existing benchmark-cache Qwen, Silero and Nemotron models. The original installed RC was restored after scratch verification.

## Integrated source

- Integration branch: `integ/mac-1.0`, advanced by local fast-forward from `codex/mac-windows-integration`. Both worktrees were clean after integration.
- Verified base: `0ea5bd252c48c7796ae82647101b07e1fefc1b10`.
- Imported bundle tip: `df38fc42a84d7053480965375aaf1be39b988bc3`.
- Imported tree: `c563abf1358902dfdaf4b062404a1066a0502203`.
- Bundle SHA256: `4d5b63a908aaea4e4c994ea1d8bcfe98c504252427e7b4307354212cdf913d66`.
- Bundle verification/base availability and fetched tip/tree matched. All 21 commits integrated by fast-forward; no merge conflicts. Benchmark branch excluded.
- Mac parity commit: `469030e1`; bundler repair commit: `9df95d5f07bc96ccbcb1cb270522591c40594984`.
- Missing-audio warning repair: `4752792e`. Native notices now retain the channel, so a missing far-side stream no longer claims that neither side captured audio. The regression failed before the repair and all7 focused cases passed afterward.

## Mac parity

- Remembered-app removal: confirmation describes inherited default, rediscovery and retained past recordings; changed default reconfirms; unreadable store refuses; correlated persisted acknowledgement removes the row.
- Meeting shortcut: separate Mac event tap, default Off, persisted native setting/state, independent toggle. Recorder requires both correlated suspension acknowledgements, waits for key/modifier release before successful rearm, resumes on cancellation/failure/timeouts, clears on core stop and preserves three-way conflicts and stored keys. Voice edit uses the existing recorder beside AI.
- Mode Save: approval is independent of the global polish switch and bound to the exact editor, model, destination, provider model and name override. Revalidation before Allow and after reply, exact confirmed pin, revoke behavior and stopped-core editor recovery preserved.
- Imported shared prompt/live-offline changes compile on Mac. Qwen local context is 8192; Foundation Models retains its independently reported context/default4096. FluidAudio was not replaced or retuned; Swift engine API unchanged.
- Orb strength10–100 by tens/default70, renderer uniform boost, shared fading and accessibility caps. Recording pill has left orb, larger centered app, separate white REC/coral dot, palette wash and visible fallback. Consent/warning layouts and nonactivation preserved; action contrast corrected in both modes.
- Native Settings phrase hierarchy, persistent form labels, responsive model status and expandable accuracy details retain existing source figures.
- Canonical events/shaders verified current without unnecessary regeneration. SwiftPM's exact verified build-local framework rpath is removed before existing strict linkage/signing checks; unknown absolute paths and symlink escapes remain refused.

## Verification actually run

Checks used installed caches, temporary data and the two subsequently authorized pinned dependency source downloads. No model/tool/dataset downloads, owner library/credentials/recordings/transcripts used for testing, release installation or publishing. Existing benchmark-cache models were subsequently used with credited public AMI fixtures. An initial app-name lookup automatically returned the legacy dashboard; its contents were not used or copied. Subsequent UI checks bound the exact scratch executable.

- Rust workspace gate: **1908 passed,20 ignored**,123 test executables covered. The initial run found one key-validation regression; resumed remaining binaries found three outdated conflict expectations. Each was corrected and its exact repro passed; completed unchanged binaries were not repeated. Workspace doc checks exit0.
- Rust fmt and all-targets workspace clippy: exit0. Engine-enabled llama/Silero/NeMo adapters: clippy exit0; **201 passed,38 ignored** in that configuration. Engine-enabled privacy logger regression: **1/1**.
- Swift gate: **724 passed,13 skipped**,737 final production test cases covered across the interrupted run, remaining cases and focused corrections. Initial Settings identity crash fixed and its layout repro/card checks passed3/3. Old blot positive-control threshold reconciled with intended shader radius/softness: background edge20/full93 in both themes, strict background limit retained. Exact mode/recorder regressions49/49. Four failures in the original Rust attempt and the interrupted Swift failure are preserved in raw evidence, not described as initial green runs.
- Swift skips: four AMI/live real-model cases; three Foundation Models opt-in cases; four optional icon/Drop/Stats export cases; GPU timing opt-in; onscreen frame-rate case (window not on screen). No TSan run: changed Swift models remain MainActor-isolated; targeted lifecycle/acknowledgement regressions ran. Swiftlint/swift-format are unavailable and were not installed.
- Mac script gate:17 existing scripts/509 assertions passed; new real-Mach-O rpath regression8 assertions passed. Rust licence/bans/sources and Swift pins/checkouts/binaries licence audits pass; Rust notices current.
- Canonical native debug and release engine builds succeed. The subsequent macOS26 engine release build passes strict linkage with no newer-macOS allowance:95 Mach-O files,89 loose engine libraries and one framework; every file targets26.0 or older. Signature/deep strict verification, hardened runtime/audio-input/calendar entitlements and designated requirement matching the existing RC pass. Debug-only replay/cloud hooks absent from release as checked by bundler.
- macOS26 dependency rebuild: the canonical script verified both authorized archive hashes;96 Abseil/SentencePiece and89 NeMo prefix dylibs independently checked for arm64/minimum26.0, signatures and manifest hashes. NeMo/ggml local pinned sources copied to scratch; existing prefixes/source unchanged. Rebuilt-library regressions:7 passed,10 ignored, plus the explicit installed-library ggml isolation case1/1. Full unchanged suites were not repeated.
- Cached Qwen/NeMo real-model interoperation:1/1 passed in19.15 seconds, both engines answering in one process against public AMI audio. Signed debug app replay of the repository's credited30-second mic/far AMI fixtures finalized: mic3 segments/20 words; far4 segments/32 words/3 speakers. Raw database rows were inspected for these counts; transcript text was not opened/exported. Live partials remain untested because Parakeet was not staged; summaries stayed Off.
- Native input probe: existing Accessibility/event-posting permissions granted. Repository input-check harness inserted once into a disposable TextEdit document in392ms, preserved clipboard item fingerprints and left secure input off. This verifies platform insertion separately from the public-audio ASR replay; a single physical microphone-to-insertion interaction remains untested. Selection-read probe timed out; no claim of successful selection reading. A bounded hardware shortcut harness received no cycle from routed UI input, which does not exercise the OS event tap.
- Shortcut focus-loss check: recording disabled both shortcut controls; activating the disposable TextEdit window cancelled recording and restored the enabled controls and prior keys.
- Specialist Swift/Rust/security briefs applied once to risky changes; findings resolved and checked. No unresolved critical/high review findings.

Raw checks remain locally under `integration-evidence/` (excluded from git). Input Windows evidence remains in the extracted scratch handoff.

## Visual evidence and limits

- Whole Settings offscreen Light/Dark at720/1040: `integration-evidence/settings-renders/`. Card bounds and window width assertions pass.
- Pill native Metal composition/fallback, manual/automatic/offer/long app names in both modes, plus narrow Appearance/Models/Voice crops: scratch `renders/` accompanying the RC manifest. Recording status, warning text/actions and corrected button contrast inspected.
- Offscreen AppKit images exclude some blur/Metal layers; native orb images were composited with the actual installed Metal shader. Additional live scratch Light/Dark Appearance screenshots and Dark voice-command hierarchy are in the chat tool record. Presets realized correctly; orb80 and the recorded meeting shortcut persisted after restart. Escape restored enabled shortcut controls; recording a conflicting dictation chord preserved both keys and showed the canonical refusal. Voice-edit consent cancellation left edit Off and resumed the controls. Remembered-app removal showed inheritance/rediscovery/past-recording wording and removed the row after acknowledgement. Microphone test detected32 percent, stopped, and reported “Inkwell heard you.” A short meeting start/stop exposed a misleading missing-audio message; channel preservation was repaired and regression tested. Subsequent real-model public-fixture replay finalized successfully. Actual live recording pills were inspected in Light and Dark: left Metal orb, centered meeting name, coral dot and white REC label. Installed-model badges and expanded accuracy details were inspected with the cached models. No retained scratch audio or transcript text was opened/exported. Existing layout constraint warnings occurred while bounds assertions passed.
- The original RC remains preserved with its macOS27-only dependency limitation. The new RC removes that binary-minimum limitation through pinned-source rebuilds and passes strict26.0 linkage. The engine build and warning-fix debug replay start on the current macOS27 host; the final warning-fix release passed the canonical build/signing gate. Runtime on macOS26 itself is still untested. No privacy, linkage or signing refusal was bypassed.

## Local RC artifact

- New archive: `Inkwell_1.0.0_mac-integration-macOS26-local-RC-v2.zip` (scratch output only, not installed); ZIP integrity passes.
- SHA256: `aa4b137116dee104b3dc90be59da14f32c34bc8d5393a88bd0c791b493da5eb0`.
- Source commit `4752792e1016f4eb50715decfa33a1b68eb9620f`; source/options in adjacent `RC-MANIFEST-macOS26-v2.json`; no signing identity recorded. Documentation-only follow-up does not change the product.
- Earlier macOS27-only archive and pre-warning-fix macOS26 archive retained separately with their manifests.
- Signed with stable local identity, no timestamp/notarization/DMG/public release.

## Windows identity and deferred scope

Supplied Windows handoff, not rerun on Mac: installer `Inkwell_1.0.0_x64-setup.exe`, source `df38fc42a84d7053480965375aaf1be39b988bc3`, SHA256 `9875eadd6443e9db5504141484ee17aa53c9ac85c31b86bb894b9f6a88ec0c0e`. Mac integration does not change that installer. Windows .NET/WinUI and manual runtime checks were not verified here. Windows checks1/2 are waived, not passed. WhatsApp remains deferred and excluded. Spanish benchmark wording, narrow Windows greeting and inactive undo/pause remain supplied nonblocking notes.

No push, tag, PR, publishing, deployment or release installation. Original working changes preserved.
