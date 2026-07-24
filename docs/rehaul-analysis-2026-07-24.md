# Inkwell — Complete Rehaul Analysis

> **Import note (added when this analysis was copied into the repo, 2026-07-24).** The document is otherwise unchanged. One of its open decisions has since been settled by the owner: §8.1 recommends a paid one-time model, and §5 argues a free MIT build has no defensible position. **The decision went the other way: Inkwell stays free, open source and BYOK, funded by an optional donation link.** Read §5 and §8 as the market case that was heard and overruled, not as the current plan. The current plan is [../PRD.md](../PRD.md) and [../TODO.md](../TODO.md). Everything else here (the code analysis, the bug list, the phased plan) still stands.

*2026-07-24. Produced from a 9-agent verified review: product, v1 UI, v1 backend, v2 audit, v1↔v2 delta, 2026 market landscape, two adversarial verification passes (166 claims confirmed, 20 adjusted, 0 fabricated), completeness critic. All file:line references verified against the code.*

---

## 0. Verdict (TL;DR)

**Rehaul v1 in place, Mac-first. Park v2's code; keep v2's docs as the refactor contract.**

- v1 (`projects/_archive/inkwell`) is a real shipped product: end-to-end verified dictation, 60 passing pipeline tests, macOS CI artifacts, and the entire brand/UI. Its debt is concentrated in **4 files (~1,900 lines)**; the other ~2,700 Rust lines and the whole frontend are already clean.
- v2 (`projects/aitappers/inkwell-v2`) is an architecture essay, not a rebuild: 1,088 lines, ~a quarter to a third dead ceremony, its one implemented flow is fake (transcribes ~10ms of audio and returns a placeholder string), zero tests, **no git repo** (gitignored inside the aitappers site repo), Windows-verified only. Rebuilding to v1 parity from it means rewriting ~95% of the product. Its genuinely good artifacts are `AGENTS.md` and `docs/architecture.md` — a solid refactor spec.
- Reality check that changes everything: **the shipped v1 had ~35 total downloads, 1 star, 0 forks, 0 issues.** This is not a relaunch with users to migrate; it's a first launch with a finished prototype. That kills migration concerns, makes the free-proxy abuse risk theoretical, and means the real bottleneck was never architecture — it was **distribution** (unsigned installers, workers.dev endpoints, no domain, no marketing).

---

## 1. What Inkwell is (product analysis)

**Thesis (PRD v0.3):** premium local-first dictation for non-technical knowledge workers. "Premium + Private + Cross-platform + Free. Nobody is here." Target user "will leave if the UI feels open source." Progressive disclosure via Advanced Mode.

**What actually shipped in v0.1.0/v0.1.1 (Mar 31 / Apr 1, 2026)** — far ahead of its own roadmap; most of the planned v0.2/v0.3 surface shipped on day one:

- Core loop: hotkey (PTT + toggle) → cpal capture → rubato resample → Silero VAD → sherpa-onnx STT (15s chunks, 0.5s overlap) → style formatter → dictionary → snippets → optional LLM polish → clipboard + synthetic paste
- 13 local STT models (Parakeet v2/v3, Moonshine, 8 Whisper variants, SenseVoice) with in-app model manager
- AI Polish: BYOK (OpenAI/Groq/Anthropic/OpenRouter/custom) + free 4,000 words/week via a personal Cloudflare Worker proxy
- Snippets with variable interpolation, voice commands ("inkwell" wake prefix), custom dictionary, per-app style overrides (Windows-only)
- File transcription (symphonia, 12 container formats) + TXT/SRT/JSON/CSV export
- SQLite history, tray, transparent always-on-top overlay, 5-step onboarding, WebGL ink shader identity
- Updater (minisign pubkey pinned client-side — correct trust design), Voice Agent mode wired to OpenClaw
- 60 pipeline tests (text-processing layer only)

**Never shipped:** streaming/live transcription (researched to plan depth in `research/raw/08-streaming.md`, never built), chainable transforms, portable mode, code signing on either platform, package managers, verified end-to-end auto-update.

**Structural contradictions found:**

1. **Distribution contradicts the thesis.** A "premium for non-technical users" product that requires bypassing SmartScreen on Windows and running `xattr -cr` in Terminal on macOS, downloaded from GitHub Releases, with the homepage on a vercel.app subdomain and updater/proxy on personal workers.dev URLs. This alone caps the audience at hobbyists.
2. **No business model.** PRD says "Closed source, free (v1). Future: free core + premium tier" (PRD.md:19) — but the repo shipped MIT and public, and no premium tier was ever defined. Note (verified by the critic): with sole copyright, zero forks, and zero outside contributors, **you can relicense all future versions closed/paid at will.** The MIT v0.1.1 snapshot has no competitive weight.
3. **Platform inversion.** Both codebases were only ever verified on Windows; you now work on macOS — which is where the entire paying dictation market lives (superwhisper, VoiceInk, MacWhisper, Wispr's core base).
4. **Agent mode is dead on arrival.** Hardwired to the OpenClaw gateway (agent.rs:7), which you decommissioned 2026-06-15. It occupies a global hotkey, a settings tab, pipeline branches, and sounds.

---

## 2. What you did graphically (v1 UI analysis)

The frontend is 4,135 lines of React 19 + Tailwind v4 + framer-motion, with a genuinely distinctive identity and a messy last mile.

### The identity (keep — this is the brand)

- **Design language:** dark-only 4-elevation charcoal (#0e0e11 → #2a2a32), white-alpha text/border ramps, one warm copper accent (#c8956c), Geist Sans display / Inter body / Geist Mono metadata.
- **The signature move:** the left ~35% of the window is a **cream-background WebGL panel (InkCanvas)** rendering a dark simplex-noise ink blob that reacts to live 3-band FFT audio (80–300 / 300–2k / 2k–8k Hz) with lerp smoothing and film grain, INKWELL wordmark in `mix-blend-difference` over it. No competitor has anything like it.
- **Overlay:** separate 97px transparent always-on-top click-through webview with an organic canvas-2D ink blob (fbm-deformed outline, specular highlight, grain). The macOS transparency path is already solved (cocoa `clearColor` hack, fixed in 0.1.1).
- **Motion discipline:** one spring vocabulary reused everywhere (stiffness 260–500, damping 24–35), `layoutId` shared tab underline, AnimatePresence menus/toasts, 0.12s crossfades, 15ms staggers.
- **Onboarding:** 5-step gated wizard — welcome, mic picker, model download offer, **forced hotkey test that echoes back the user's real first transcription ("It worked!")**, tray explainer. One of the strongest UX pieces in the app.
- **AI consent modal:** honestly explains what leaves the device, shows quota upfront, easy decline. Dark-pattern-free.
- **GeneralTab style cards:** live example sentences with mirrored label typography ("Formal." with period, "very casual" lowercase). Great progressive-disclosure moment.

### The rot (fix or delete)

| Issue | Evidence | Severity |
|---|---|---|
| ~527 lines (12.7% of src) dead: unimported App.css + stale duplicates of Onboarding/TabBar/StatusBar (newer copies live inline in the 645-line App.tsx; the duplicates already diverged once) | src/components/*, src/App.css | major |
| **Permanent `getUserMedia` stream from app start** — constant macOS orange mic indicator + second concurrent capture beside cpal. Reads as spyware for a privacy-branded app | InkCanvas.tsx:183-241 | critical |
| AgentTab broken twice: wrong invoke argument shape AND no `agent_*` match arms in the backend — nothing it saves can persist. Targets dead OpenClaw | AgentTab.tsx:17-25 vs commands.rs:129-153 | critical |
| 12 horizontal tabs is the wrong IA: at default 800px roughly half the advanced tabs hide behind a 3-dot overflow menu; basic/advanced tab-set swap prevents a spatial map | types.ts:80-82, App.tsx:275-278 | major |
| No state store despite shipping zustand: every tab independently refetches `get_settings` and writes per-field; ~30 empty `.catch(() => {})` sites swallow errors silently | all tabs | major |
| Windows residue: "Ctrl + Space" onboarding copy (conflicts with macOS input-source shortcut), metaKey rendered as "super", notepad.exe/slack.exe placeholders, process-name (not bundle-ID) per-app matching | App.tsx:167, GeneralTab.tsx:20, AppStylesTab.tsx:65 | major |
| Dead deps: 7 of 8 Radix packages, zustand, geist npm; Inter AND Geist both loaded | package.json:14-33 | major |
| Micro-typography: dominant 10–13px via 40+ arbitrary `text-[Npx]` values, no scale — the single biggest "dated vs premium" factor in screenshots | all tabs | major |
| Overlay soundwave bars are **fake** (pure sine of time) while the main canvas is genuinely audio-reactive — the surface users see during every dictation is the one that lies | public/overlay.html:127-131 | minor |
| Version drift shipped: status bar hardcodes v0.1.0, About says v0.1.1 | App.tsx:606, AboutTab.tsx:9 | minor |
| Destructive actions without confirm (transcript Del is immediate), `alert()` in ModelsTab, hover-only invisible Del/Copy buttons, unlabeled 8px toggle dots | DashboardTab.tsx:110-114, ModelsTab.tsx:67 | minor |
| "Glass" kit is a misnomer (no blur, solid surfaces) and only 6 of 12 tabs use it; the homepage uses real 24px backdrop blur — marketing and product disagree on the core material | GlassSurface.tsx:10-16 vs homepage globals.css:24-35 | minor |

### UI weight distribution

App.tsx 645 · AITab 360 · VoiceCommandsTab 256 · DashboardTab 239 · GeneralTab 228 · FilesTab 220 · ModelsTab 214 · SnippetsTab 165 · AgentTab 164 · AppStylesTab 115 · AudioTab ~100 · rest smaller.

### UI direction for the rehaul (adjudicated)

Keep the ink identity **verbatim** (shader + band extraction + smoothing constants, overlay blob renderer, token block, motion constants, onboarding structure, consent modal, style cards — all self-contained, near-zero port cost). Replace the 12-tab IA with a **grouped sidebar** (History / Dictation / Intelligence / System) in the main window — but recognize the daily-driver surface of this product is the **tray + overlay HUD**, which v1 already has and which should get the real-audio fix. Don't rebuild as a popover-only menu-bar app; do make the main window something you open weekly, not daily. Consider real macOS translucency (macOSPrivateApi already enabled) to resolve the glass question in the homepage's favor.

---

## 3. v1 backend analysis

4,933 lines of Rust across 26 files + two deployed Cloudflare Workers (polish proxy, update server).

### The good (port-worthy, largely verbatim)

- **The engine seam is the crown jewel:** `engine.rs`, `vad.rs`, `recording.rs`, `filetranscribe.rs` are pure functions over `&[f32]` with zero AppState coupling. They encode months of hard-won quirks: Moonshine V1/V2 layout detection, int8/non-int8 file-candidate fallbacks, Parakeet's ~20s limit (15s chunks + 0.5s overlap), device workarounds. Rewriting these re-derives known bug fixes.
- **Text-processing quartet** (style/dictionary/snippets/voicecommand): pure, portable, covered by the 658-line `pipeline_tests.rs` (the only meaningful tests).
- **llm.rs provider trait + factory:** clean BYOK abstraction over 5 providers.
- **Updater trust design:** minisign pubkey pinned client-side; worker/KV compromise can't ship a malicious update.
- Sensible privacy defaults: polish off by default, keys in OS keyring, everything local.

### The debt (the legitimate reason v2 exists — confined to 4 files, ~1,900 lines)

- **AppState: 22 Mutex fields**, 123 `lock().unwrap()` sites — one poisoned mutex cascades panics app-wide (lib.rs:30-57).
- **commands.rs: 913-line god file** embedding the model registry as ~6 separate hardcoded match statements (install-check, switch, download URLs/sizes, remove, + engine.rs constructors + setup.rs autoload) — six places to touch per new model.
- **pipeline.rs:** entire dictation pipeline inline in the global-shortcut handler closure; a new tokio Runtime + nested thread + `join()` constructed per polish/agent call (duplicated in agent.rs).
- **setup.rs:** 284-line god function.
- Sync commands on the main thread: `switch_model` loads a ~700MB ONNX encoder while the UI freezes; `transcribe_file` holds the engine mutex for the whole file, blocking hotkey dictation.

### Real bugs (fix regardless of strategy)

| Bug | Location |
|---|---|
| Saved model never restored at launch — setup hardcodes Parakeet-else-Moonshine, ignoring `settings.model` | setup.rs:169-214 |
| Interrupted model download permanently wedges — `exists()` check skips redownload of truncated files, no checksums | commands.rs:651-663 |
| **Agent token ALWAYS written plaintext to settings.json** even when keyring succeeds ("Always save to settings as backup") | commands.rs:856-861 |
| **Raw audio of every dictation dumped unencrypted to temp** (`inkwell_debug.wav`) in the production path | pipeline.rs:265-266 |
| Three persisted settings never honored: `mic_device`, `start_on_boot`, `show_overlay` | commands.rs |
| CSP disabled (`null`) + broad clipboard/fs capabilities to both windows | tauri.conf.json:26 |
| Chunk-overlap concatenation without dedup → repeated words at 15s boundaries | engine.rs:270-302 |
| Paste clobbers user clipboard, no save/restore; 100ms/50ms sleeps as synchronization | paste.rs:13-45 |
| Proxy rate limit keyed on client-generated install_id with CORS `*` — Groq quota burnable by anyone (theoretical at current adoption) | inkwell-worker/src/index.ts:86-111 |
| Silero VAD detector rebuilt from disk every dictation; `play_tone` blocks its thread | vad.rs:8-26, sounds.rs:147-162 |
| Dead code: 3s standby ring buffer (consumer dropped), AudioState.rms, `download_parakeet`, hand-rolled calendar math ×3 and hand-written JSON while chrono/serde_json sit in deps | audio.rs, usage.rs, export.rs |
| Cargo.toml still scaffold metadata: "A Tauri App", authors ["you"] | Cargo.toml:4-7 |

### macOS readiness (both codebases)

v1 is **less Windows-bound than expected**: paste already has a Cmd+V branch, overlay transparency solved via cocoa, icns shipped, CI already produced mac DMGs. Genuinely Windows-only: `appdetect.rs` (Win32 FFI, macOS stub returns None — per-app styles silently no-op), the Logitech "AI noise" device hack + WASAPI naming, nvidia-smi probe, PowerShell-only model bootstrap.

**Blocking gaps on Mac (in BOTH codebases):**
- No `NSMicrophoneUsageDescription` / Info.plist customization → bundled app gets mic-killed by TCC
- No Accessibility-permission flow → enigo synthetic Cmd+V silently fails until granted (the #1 support-ticket class for this app category)
- Unsigned/unnotarized → Gatekeeper `xattr` workaround was the documented install path
- Untested failure modes no analyst initially caught: **macOS Secure Input** (synthetic paste silently fails in password fields/terminals), **Bluetooth/AirPods mic** sample-rate degradation. A written mac QA checklist should be a deliverable.

---

## 4. v2 audit (honest verdict)

1,088 lines total (~700 Rust, ~300 TS). The docs are the best part — `AGENTS.md` and `docs/architecture.md` (11-stage pipeline, "no product logic in Tauri commands", "no plaintext secrets") are a genuinely good refactor spec, and the README's "Still unverified" list is honest discipline.

The code is another story:

- **The one implemented flow is fake end-to-end:** `start_dictation` is synchronous, emits Capturing→Normalizing→Transcribing→Completed back-to-back with zero elapsed work, captures only the **last ~10ms cpal callback chunk** (the callback replaces the buffer instead of accumulating — audio.rs:69-73), and transcribes via a placeholder returning the literal string "placeholder transcription". No stop/cancel, no recording-session concept.
- **It violates its own core rule:** the entire pipeline orchestration lives inline in the shell command handler (commands.rs:50-113) — exactly what AGENTS.md:39 forbids. No app-layer DictationService exists.
- **Mic hot from launch:** stream built and `.play()`ed in bootstrap, held forever — same orange-indicator problem as v1's frontend, in the layer that was supposed to fix such things.
- **Lifecycle holes:** error paths strand jobs at Capturing/Transcribing; `JobStage::Failed` is never constructed anywhere; frontend silently drops every intermediate job event (events arrive before the job exists in the store; `updateJobStage` no-ops on unknown ids) — the event plumbing demonstrably does nothing today.
- **~24–35% dead ceremony:** CaptureService, Capability, ModelType, Transcript, TransformStage, AppReadyEvent, SecretStorePort.has_secret, SettingsStore.save, settings-store.ts, DictationPanel.tsx (`return null`, never imported), unused `hound` dep.
- **Zero UI.** No CSS file exists; App.tsx is a debug console dumping JSON.
- **No product-promise dependencies even declared:** no global-shortcut, tray-icon, clipboard, enigo, rusqlite, keyring, reqwest in Cargo.toml. v2's contracts cover ~6 commands vs v1's ~40.
- **No version control:** gitignored inside the aitappers site repo (.gitignore:49). Zero history, zero backup, one `rm -rf` from gone.
- Latent landmine: `unsafe impl Send/Sync` on the Sherpa transcriber with no safety argument — serialized today (non-async command on main thread) but becomes real UB exposure the moment commands go async.
- Correction from verification: the sherpa-onnx build is NOT a cmake/C++ problem — `sherpa-onnx-sys` downloads prebuilt static libs. The real mac question is whether a prebuilt darwin-arm64 lib exists for 1.12.x.

**Salvage value:** the docs, the module-layout idea, the contracts discipline as a rule. The runtime slice needs rewriting, not extending.

---

## 5. Market landscape (2026)

| Product | Model | Price | Notes |
|---|---|---|---|
| Wispr Flow | cloud-only | $12–15/mo | 4 platforms, streaming partials, context-aware tone, Command Mode |
| Aqua Voice | cloud-only | $8/mo | sub-50ms start claims |
| Willow / Monologue | cloud-only | $15/mo | Monologue is screen-aware |
| superwhisper | local, Mac-first | $8.49/mo / $249 lifetime; free tier unlimited small models | Modes system, Super Mode reads screen/clipboard |
| VoiceInk | local, Mac-only, GPL Swift | $25–49 one-time | whisper.cpp, ~4.3k stars |
| MacWhisper | local, Mac-only | €59 lifetime | file-transcription workhorse |
| **Handy** | **local, free MIT** | free | **27.4k stars, biweekly releases, EXACT same stack: Tauri+React+Rust, Parakeet V3, Silero VAD, cpal** |
| Apple (macOS 26) | built-in | free | SpeechAnalyzer public API: 2.12% WER LibriSpeech-clean, beats Whisper Small on speed and accuracy, ~30 locales |

**The strategic picture:**

- **Free-local-basic is gone.** Handy owns it with the identical stack and 27.4k stars. A free Inkwell has no defensible position — do not ship a free MIT clone.
- **Cloud-subscription is a funded knife-fight.** Not your lane as a solo builder.
- **The open lane:** local-first WITH the premium context layer. Every context-aware product (Wispr/Aqua/Willow/Monologue) is cloud-only; the local tools with polish are Mac-only Swift or lack the layer entirely (Handy has no streaming partials, no LLM polish). One-time pricing in the proven **$29–59 band** (VoiceInk/MacWhisper), superwhisper's $249 as ceiling.
- **2026 table stakes:** streaming partial text while speaking; per-app context-aware formatting; custom vocabulary; modes with auto-activation; VAD auto-stop; credible free tier.
- **Engine:** keep sherpa-onnx. Parakeet TDT 0.6B v3 int8 as final-pass default (6.32% WER avg, ~3333× RTFx, 25 languages). BUT Parakeet is offline-only in sherpa-onnx (issue #2918) — streaming partials require a second model. **Unresolved (needs a spike):** the v1 research plan (raw/08) targets streaming zipformer (supported, weaker); Moonshine v2's ergodic streaming encoder is preferable on paper but sherpa-onnx runtime support is unverified. On macOS, FluidAudio's Parakeet v3 CoreML (~110× realtime on ANE) and Apple SpeechAnalyzer are both reachable only via a small Swift sidecar — worth it as an optional "Apple engine" tier (zero-download first-run kills the biggest onboarding drop-off), not as default.
- **Risk:** raw local STT is commoditizing (Apple ships it free). The moat must be the workflow layer — modes, polish, vocabulary, history, command mode — not transcription itself.

---

## 6. Keep / Cut / Add

**KEEP (the product):** core dictation loop, VAD, style formatting, custom dictionary, transcript history + search, tray + overlay, onboarding, file transcription + export, model manager (curated), ink-shader identity, both Cloudflare Workers as deployed infra (updater as-is; proxy repurposed or shut), the 60-test corpus, engine seam verbatim.

**CUT:**
- **Agent mode entirely** (agent.rs, AgentTab, hotkey branch, sounds) — OpenClaw is decommissioned; the tab never persisted its settings anyway. If voice-to-agent returns, target a generic OpenAI-compatible/MCP endpoint.
- **Model catalog 13 → 3-4** (bundled tiny fallback, Parakeet v3 default, one Whisper multilingual, maybe SenseVoice); shelve the FireRed/Omnilingual expansion research — wrong direction for the stated user.
- **Free-proxy tier as shipped** — BYOK-only at first; rebuild later as the paid tier's metered endpoint on an owned domain if going freemium.
- Snippets + voice commands from default visibility (Advanced-only; snippets duplicate OS text expanders).
- Chainable transforms, portable mode, GPU/CUDA plumbing (wrong bet on Mac — CoreML/ANE is the path), Meeting mode / Teams-Enterprise roadmap entries.
- All dead code: v1's ~527 frontend lines + duplicates, dead deps, ring buffer, `download_parakeet`; v2's ceremony files.

**ADD:**
- **Streaming partials** (the headline gap) — after a 1–2 day engine spike (zipformer vs Moonshine v2 vs Apple-engine partials).
- **macOS as first-class:** Info.plist mic description, guided Accessibility+mic permission onboarding (tauri-plugin-macos-permissions), NSWorkspace bundle-ID appdetect, signing + notarization in CI.
- **Distribution as a feature:** owned domain, Developer ID ($99/yr) + Windows signing (Azure Trusted Signing ~$10/mo beats OV certs), Homebrew cask, updater endpoints off workers.dev.
- **A business model decision** (see §8).
- Opt-in privacy-safe telemetry or at minimum download counts + feedback channel — v1 launched blind.
- Mac QA checklist: TCC grant/revoke, Secure Input fields, AirPods mic, multi-display overlay.

---

## 7. The plan (strategy B, sequenced)

**Phase 0 — housekeeping (half a day)**
1. Un-archive v1 → active project dir; import v2's AGENTS.md + architecture.md as `docs/architecture-target.md`; archive v2's code (or `git init` it purely as reference); this analysis moves with it.
2. Secure the minisign updater private key (currently only a GitHub Actions secret in the archived repo — losing it breaks the update chain permanently).
3. Fix the two privacy leaks now: keyring-only agent token (+ purge from settings.json), gate the debug WAV behind a flag.

**Phase 1 — Mac-first platform pass (the unlock)**
4. `cargo tauri dev` green on this Mac; Info.plist mic description; Accessibility onboarding step (AXIsProcessTrusted check before first paste); delete Logitech hack + nvidia-smi; menu-bar template icon; kill the permanent getUserMedia (drive InkCanvas from Rust-emitted band events — fallback path already exists at InkCanvas.tsx:174-180).
5. Signing + notarization in the existing build.yml; then flip the updater live (darwin manifest slots already exist).

**Phase 2 — strangle the debt (keep 60 tests green throughout)**
6. Extract `process_recording` into a staged async service (v2's 11-stage doc is the spec); split commands.rs by domain; model registry → single data table + checksum/.partial downloads; AppState → few coarse services on Tauri's managed runtime (`spawn_blocking` for inference); adopt typed contracts command-by-command (tauri-specta).
7. Fix the bug list from §3 (model restore, download wedge, unhonored settings, CSP, overlap dedup, clipboard restore).

**Phase 3 — UI rehaul**
8. Sidebar IA, one zustand settings store + single error-toast path, kit unification + real type scale (13–14px floor), macOS-native input surfaces (Cmd symbols, bundle IDs, new default hotkey — Ctrl+Space collides with macOS input-source switching), real audio into overlay bars, a11y fixes, delete AgentTab + dead files/deps.

**Phase 4 — the differentiator**
9. Streaming-partials spike → two-pass pipeline (streaming front-end + Parakeet rescore); optional Apple-engine Swift sidecar (SpeechAnalyzer zero-download onboarding tier).
10. Homepage rebuild on the owned domain (pricing, demo GIF of streaming + ink identity, platform-detected download); launch channels (Product Hunt, HN, r/macapps); privacy policy.

---

## 8. Open decisions (yours, not code)

1. **Business model.** Recommended default: local-first premium, one-time $29–59, free trial or limited free tier — the only open lane per the market map. MIT history does NOT constrain this (sole copyright, no forks — relicense future versions at will). Decide before Phase 4; Phases 0–2 are identical under every model. If the answer is instead "portfolio piece for AI Tappers," strategy (a) — polishing the v2 case-study docs — becomes defensible and most of this plan shrinks.
2. **The name.** "Inkwell" collides with Apple's own Inkwell/Ink handwriting feature and several existing apps/crates; no domain was ever checked. Validate (or rename) before buying the domain and signing identity.
3. **The Handy question.** Differentiate strictly above it (source-available or closed premium layer) — or contribute v1's good pieces upstream and drop the standalone ambition. Recommended: differentiate; but it's a genuine fork in the road.
4. **Payment infra if paid:** license-key/activation, merchant of record (EU VAT), refunds — none exists in either codebase; weeks of work no roadmap line currently carries.

---

## Appendix — verified numbers

- v1: 4,933 lines Rust / 26 files; ~4,135 lines frontend; commands.rs 913, App.tsx 645, pipeline.rs 545, setup.rs 284; AppState 22 Mutex fields; 123 `lock().unwrap()`; 60 tests; ~527 dead frontend lines (12.7%); 13 models.
- v2: 1,088 lines; 7 fully-dead source files of ~29 (~24%, near a third counting partials); 0 tests; 0 CSS; verified Windows-only 2026-04-12; not in version control.
- Adoption: 1 star, 0 forks, 0 issues, ~35 total release-asset downloads (top asset: 6). Verified via GitHub API.
- Verification: 166 claims confirmed, 20 adjusted (all adjustments incorporated above), 0 fabricated.
