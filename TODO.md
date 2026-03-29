# Inkwell - Build TODO
*Step-by-step. Nix codes, Mattias tests and guides. Methodical and careful.*

---

## How We Work

Each task is a discrete unit. The pattern:
1. **Nix builds** the piece (writes code, configs, scaffolds)
2. **Mattias tests** (runs it, checks behavior, screenshots if needed)
3. **We review together** (what works, what doesn't, what to adjust)
4. **Mark complete** only when both are happy
5. **Commit** before moving to next task

No skipping ahead. No "I'll fix that later." Each step works before the next begins.

---

## Phase 1: Foundation (Weeks 1-3)

### 1.1 Project Scaffold
- [x] **1.1.1** Install Rust toolchain (rustup) - v1.94.0
- [x] **1.1.2** Install Tauri v2 CLI - v2.10.1
- [x] **1.1.3** Scaffold Tauri v2 project with React + TypeScript + Vite template
- [x] **1.1.4** Verify: `cargo tauri dev` opens a window with React hello world
- [x] **1.1.5** Add Tailwind CSS v4 to frontend
- [x] **1.1.6** Add Framer Motion, Zustand, Radix primitives to frontend
- [x] **1.1.7** Add fonts: Geist Sans, Inter, Geist Mono
- [x] **1.1.8** Set up base dark theme CSS variables (from design-direction.md)
- [x] **1.1.9** Verify: app opens with dark background, correct fonts, Tailwind working
- [ ] ~~**1.1.10** First commit~~ (skipped, building locally until MVP)

**Test checkpoint:** App opens. Dark background. Fonts load. Tailwind classes work. Window resizes correctly.

### 1.2 Split Panel Layout
- [x] **1.2.1** Create split-panel layout component (left ~35%, right ~65%)
- [x] **1.2.2** Left panel: dark background, full height, holds ink shader
- [x] **1.2.3** Right panel: slightly lighter background, holds content
- [x] **1.2.4** "INKWELL" wordmark top-center with mix-blend-difference
- [x] **1.2.5** Tab bar (Dashboard, General, Audio, About) with spring-animated indicator
- [x] **1.2.6** Tab switching with Framer Motion crossfade
- [x] **1.2.7** Verified: layout responsive, tabs smooth
- [ ] ~~**1.2.8** Commit~~ (skipped)

**Test checkpoint:** Two-panel layout looks correct. Tabs switch with smooth animation. Wordmark visible.

### 1.3 Glass Surface Components
- [x] **1.3.1** Glass surface component (GlassSurface.tsx)
- [x] **1.3.2** Glass card + SettingRow component (GlassCard.tsx)
- [x] **1.3.3** Glass toggle switch, spring-animated (GlassToggle.tsx)
- [x] **1.3.4** Glass input field (GlassInput.tsx)
- [x] **1.3.5** Glass dropdown, spring-open (GlassSelect.tsx)
- [x] **1.3.6** Ghost button + primary button (GlassButton.tsx)
- [x] **1.3.7** All components tested in General tab
- [x] **1.3.8** Verified: premium feel, spring animations work
- [ ] ~~**1.3.9** Commit~~ (skipped)

**Test checkpoint:** Components look and feel premium. Springs overshoot slightly then settle. Glass blur is visible on dark background.

### 1.4 Ink Shader (Idle State)
- [x] **1.4.1** WebGL canvas in left panel (InkCanvas.tsx)
- [x] **1.4.2** Vertex shader (fullscreen quad)
- [x] **1.4.3** Fragment shader: simplex noise, 2 octaves, slow drift
- [x] **1.4.4** Uniforms: u_time, u_resolution, u_amplitude, u_low, u_mid, u_high, u_state
- [x] **1.4.5** Color: cream background, dark ink blob (Symmetry Breaking reference)
- [x] **1.4.6** Film grain (2% opacity)
- [x] **1.4.7** "INKWELL" text with mix-blend-difference (inverts over ink)
- [x] **1.4.8** Verified: organic breathing movement, ink alive
- [ ] ~~**1.4.9** Commit~~ (skipped)

**Test checkpoint:** Left panel has a living ink blob that drifts slowly. Feels organic and premium. No jank or stuttering.

### 1.5 Audio Capture (Rust)
- [x] **1.5.1** cpal v0.17 + ringbuf added
- [x] **1.5.2** Mic enumeration, logged at startup
- [x] **1.5.3** Always-on audio capture (cpal stream)
- [x] **1.5.4** 3-second ring buffer (standby mic)
- [x] **1.5.5** RMS amplitude computed every ~50ms, smoothed
- [x] **1.5.6** Amplitude emitted as Tauri event "audio-amplitude"
- [x] **1.5.7** Frontend listens to amplitude, feeds ink shader
- [x] **1.5.8** Verified: ink reacts to voice from webcam mic
- [ ] ~~**1.5.9** Commit~~ (skipped)

**Test checkpoint:** Speak into mic. Ink moves in response. Louder = more movement. Silent = slow drift. This is the magic moment.

### 1.6 Web Audio Visualization (Frontend)
- [x] **1.6.1** Web Audio API getUserMedia → AnalyserNode (with Tauri event fallback)
- [x] **1.6.2** 3 frequency bands extracted (low 80-300Hz, mid 300-2kHz, high 2k-8kHz)
- [x] **1.6.3** u_low, u_mid, u_high uniforms added to shader
- [x] **1.6.4** Shader response: low = blob scale, mid = warp, high = surface detail
- [x] **1.6.5** All uniforms smoothed (lerp 0.08)
- [x] **1.6.6** Verified: frequency-reactive ink works. Mattias noted sensitivity "a bit too much" - dial back shader amplitude in future pass.
- [ ] ~~**1.6.7** Commit~~ (skipped)

**Test checkpoint:** Different sounds create different ink behaviors. Speech is smooth and responsive. Music makes it dance differently than voice.

---

## Phase 2: Core STT (Weeks 4-6)

### 2.1 Global Hotkey
- [x] **2.1.1** Tauri global-shortcut plugin added
- [x] **2.1.2** Hotkey registered: Ctrl+Space (push-to-talk)
- [x] **2.1.3** Push-to-talk mode: hold = recording, release = stop
- [x] **2.1.4** Recording state emitted to frontend ("recording-state" event)
- [x] **2.1.5** Ink shader u_state uniform: spring-animated idle↔recording transition
- [x] **2.1.6** Verified: hotkey works, ink reacts on press and release
- [ ] ~~**2.1.7** Commit~~ (skipped)

**Test checkpoint:** Press hotkey from any app. Ink visibly shifts to recording state. Release and it settles back. Works while typing in Notepad, browser, etc.

### 2.2 Recording Pipeline
- [x] **2.2.1** Hotkey press: clear buffer + start collecting (audio.rs recording_buffer)
- [x] **2.2.2** Hotkey release: stop, take buffer, spawn background thread
- [x] **2.2.3** rubato v0.16: resample to 16kHz mono (recording.rs)
- [x] **2.2.4** Save debug WAV to %TEMP%\inkwell_debug.wav (hound crate)
- [x] **2.2.5** Working. Bug fixed: Logitech X Pro "AI Noise-cancelling Input" is a cpal virtual device (produces silence). Fixed find_preferred_device() to skip "ai noise" devices and use real hardware mic.
- [ ] ~~**2.2.6** Commit~~ (skipped)

**Test checkpoint:** Press hotkey, speak, release. A WAV file appears with clear audio of what you said. Resampled to 16kHz mono.

### 2.3 Silero VAD
- [x] **2.3.1** sherpa-onnx v1.12 added (static linking)
- [x] **2.3.2** Silero VAD module (vad.rs): remove_silence()
- [x] **2.3.3** Wired into recording pipeline (lib.rs: resample → VAD → transcribe)
- [x] **2.3.4** Working. Silero VAD removes 20-45% silence. Silero VAD model at %APPDATA%\com.inkwell.app\models\
- [ ] ~~**2.3.5** Commit~~ (skipped)

**Test checkpoint:** Record with a pause before speaking. VAD trims the silence. Output audio starts at first speech.

### 2.4 Model Integration (Moonshine Tiny)
- [x] **2.4.1** SpeechEngine struct with Send/Sync (engine.rs)
- [x] **2.4.2** SpeechEngine::moonshine() constructor (finds int8/regular model files)
- [x] **2.4.3** SpeechEngine::transcribe() method
- [x] **2.4.4** Model loading at startup (checks moonshine-tiny dir)
- [x] **2.4.5** Transcription result emitted as "transcription" Tauri event
- [x] **2.4.6** Working. Bug fixed: HF repo uses V1 layout (preprocess.onnx + encode.int8.onnx + cached_decode.int8.onnx + uncached_decode.int8.onnx + tokens.txt). engine.rs now auto-detects V1 vs V2. scripts/download-models.ps1 updated.
- [x] **2.4.7** Working as fallback. Moonshine accuracy: "Hello, my name is Bautier." (wrong). Used as fallback when Parakeet not present.

**Test checkpoint:** The core magic works. Speak, get text. Accuracy won't be perfect (it's Tiny) but it should be recognizable.

### 2.5 Model Integration (Parakeet V3)
- [x] **2.5.1** In-app model downloader implemented (HTTP fetch from HuggingFace to models dir)
- [x] **2.5.2** Download progress events emitted to frontend
- [x] **2.5.3** Parakeet V3 inference working. csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8 (encoder 652MB + decoder 11.8MB + joiner 6.4MB + tokens.txt). model_type="nemo_transducer". Load time ~18-22s, 720MB RAM. Stored at %APPDATA%\com.inkwell.app\models\parakeet-v3\.
- [x] **2.5.4** Runtime model hot-swap done (SpeechEngine trait)
- [x] **2.5.5** Auto-download Parakeet on first launch (background)
- [x] **2.5.6** Auto-switch to Parakeet when download completes
- [x] **2.5.7** Verified: Parakeet is significantly better. "Hello, my name is Matthias." vs Moonshine "Bautier."
- [x] **2.5.8** Non-English speech tested (Swedish, Spanish, German)
- [x] **2.5.9** Done

**Test checkpoint:** Parakeet downloads in background. Auto-switches. Accuracy is excellent. Language auto-detection works.

### 2.6 Model Integration (Whisper)
- [x] **2.6.1** whisper-rs added to Cargo.toml
- [x] **2.6.2** WhisperEngine (SpeechEngine trait) implemented
- [x] **2.6.3** GPU detection (CUDA, Metal, Vulkan)
- [x] **2.6.4** Whisper Small tested via model manager
- [x] **2.6.5** Verified: Whisper transcription works, GPU acceleration if available
- [x] **2.6.6** Done

**Test checkpoint:** Can switch between Parakeet and Whisper. Both produce correct transcriptions. GPU is used if available.

### 2.7 Clipboard Paste
- [x] **2.7.1** Using arboard crate (not Tauri clipboard plugin) for clipboard write
- [x] **2.7.2** After transcription: write text to clipboard via arboard
- [x] **2.7.3** Using enigo v0.3 (not rdev) for keyboard simulation. Ctrl+V (Windows/Linux) / Cmd+V (macOS). New file: src-tauri/src/paste.rs
- [x] **2.7.4** 100ms delay (not 10ms) - ensures Ctrl from hotkey release doesn't interfere with paste Ctrl+V
- [x] **2.7.5** Ink shader: processing state → success pulse → idle
- [x] **2.7.6** Verified: text appears in focused app
- [x] **2.7.7** Tested in: Notepad, browser text field, VS Code, terminal
- [x] **2.7.8** Done

**Test checkpoint:** THE FULL FLOW WORKS. Press hotkey in any app, speak, release, text appears. This is the MVP core.

### 2.8 Style Formatting
- [x] **2.8.1** Formal formatter (capitalize sentences, full punctuation)
- [x] **2.8.2** Casual formatter (capitalize, light punctuation)
- [x] **2.8.3** Relaxed formatter (lowercase, minimal punctuation)
- [x] **2.8.4** Style selector in main UI (3 buttons/pills, quick toggle)
- [x] **2.8.5** Selected style applied before paste
- [x] **2.8.6** Verified: same speech produces different output per style
- [x] **2.8.7** Done

**Test checkpoint:** Toggle between styles. Formal gives proper sentences. Relaxed gives lowercase stream. Casual is in between.

---

## Phase 3: Polish (Weeks 7-9)

### 3.1 Settings UI
- [x] **3.1.1** General tab: hotkey config, recording mode (toggle/push-to-talk), language, startup
- [x] **3.1.2** Audio tab: mic selector (dropdown of available mics), VAD sensitivity slider
- [x] **3.1.3** Models tab: list installed models, download buttons, current default indicator
- [x] **3.1.4** About tab: version, "Inkwell by Mattias H.", links
- [x] **3.1.5** Settings persistence (save to JSON, load on startup)
- [x] **3.1.6** Verified: all settings save and restore correctly across restarts
- [x] **3.1.7** Done

**Test checkpoint:** Change settings, close app, reopen. Everything remembered. Mic selector shows real devices.

### 3.2 Transcript History
- [x] **3.2.1** rusqlite added, transcripts table on first run
- [x] **3.2.2** Every transcription stored (text, raw_text, style, model, duration, timestamp)
- [x] **3.2.3** Dashboard tab: scrollable transcript feed, newest first
- [x] **3.2.4** Each transcript: timestamp, text preview, copy button, delete button
- [x] **3.2.5** Click to expand full transcript, edit inline
- [x] **3.2.6** Search bar: filter transcripts by text content
- [x] **3.2.7** Framer Motion: staggered spring entrance for list items
- [x] **3.2.8** Verified: transcripts accumulate, search works, copy works
- [x] **3.2.9** Done

**Test checkpoint:** Do 10 transcriptions. All appear in dashboard. Search finds them. Copy pastes correct text. Delete removes them.

### 3.3 System Tray
- [x] **3.3.1** Tauri tray-icon plugin added
- [x] **3.3.2** Tray icon: idle state (ink drop icon)
- [x] **3.3.3** Tray icon: recording state (changes color/animation)
- [x] **3.3.4** Tray menu: Start/Stop recording, Style selector, Open Inkwell, Quit
- [x] **3.3.5** Click tray icon: toggle main window visibility
- [x] **3.3.6** Verified: tray works when main window closed, hotkey still works
- [x] **3.3.7** Done

**Test checkpoint:** Close main window. App lives in tray. Hotkey still records. Tray shows recording state. Can reopen window from tray.

### 3.4 Floating Overlay
- [x] **3.4.1** Secondary Tauri window (small, always-on-top, transparent)
- [x] **3.4.2** Minimal pill (recording dot + timer + mini audio bars)
- [x] **3.4.3** Shows when recording starts (if main window not visible)
- [x] **3.4.4** Hides when recording stops
- [x] **3.4.5** Overlay draggable
- [x] **3.4.6** Setting: overlay position (corner select) or disabled
- [x] **3.4.7** Verified: appears during recording, doesn't steal focus
- [x] **3.4.8** Done

**Test checkpoint:** Hide main window. Press hotkey. Small pill appears showing recording state. Doesn't interfere with typing in other apps.

### 3.5 Model Manager UI
- [x] **3.5.1** Models tab: all available models with specs (size, speed, accuracy, languages)
- [x] **3.5.2** Download button with progress bar per model
- [x] **3.5.3** Delete button for downloaded models (except bundled Tiny)
- [x] **3.5.4** "Set as default" button
- [x] **3.5.5** Current model indicator in status bar
- [x] **3.5.6** Auto-download Moonshine V2 Medium alongside Parakeet
- [x] **3.5.7** Verified: can download, switch, delete. Default persists.
- [x] **3.5.8** Done

**Test checkpoint:** Models tab shows all options. Download a Whisper model, switch to it, transcribe, switch back. Delete works.

### 3.6 Advanced Mode Toggle
- [x] **3.6.1** "Advanced Mode" toggle in General settings
- [x] **3.6.2** When off: AI, Tools tabs hidden. Audio/Models simplified.
- [x] **3.6.3** When on: all tabs, all settings visible
- [x] **3.6.4** Setting persisted
- [x] **3.6.5** Framer Motion reveal animation for new tabs/settings
- [x] **3.6.6** Verified: toggling feels clean, nothing breaks
- [x] **3.6.7** Done

**Test checkpoint:** Default: 4 tabs (Dashboard, General, Audio, About). Toggle Advanced: additional tabs appear with animation.

### 3.7 Custom Dictionary
- [x] **3.7.1** Dictionary settings UI (Advanced Mode): list of find/replace pairs
- [x] **3.7.2** Add/edit/delete entries
- [x] **3.7.3** Apply dictionary replacements after transcription, before paste
- [x] **3.7.4** Persist to dictionary.json
- [x] **3.7.5** Verified: corrections apply correctly
- [x] **3.7.6** Done

**Test checkpoint:** Add "Mattias" correction. Transcribe name. It comes out right.

---

## Phase 4: Ship (Weeks 10-12)

### 4.1 Error Handling
- [x] **4.1.1** Mic disconnected mid-recording: graceful recovery, notify user
- [x] **4.1.2** Model fails to load: fallback to Tiny, notify user
- [x] **4.1.3** Paste fails (secure input field): show toast with text, user can copy manually
- [x] **4.1.4** Model download fails: retry with exponential backoff, show error
- [x] **4.1.5** App crash recovery: state saved, resume cleanly
- [x] **4.1.6** Done

**Test checkpoint:** Unplug mic during recording. Delete a model file manually. Try pasting into a password field. Nothing crashes.

### 4.2 First-Run Experience
- [x] **4.2.1** First launch detection (no settings.json exists)
- [x] **4.2.2** Mic permission request with clear explanation
- [x] **4.2.3** Quick hotkey test ("Press Ctrl+Shift+Space to test")
- [x] **4.2.4** Model status: "Using quick start model. Downloading better model..."
- [x] **4.2.5** Celebrate first successful transcription (subtle UI feedback)
- [x] **4.2.6** Accessibility permission prompt on macOS (with explanation)
- [x] **4.2.7** Verified: brand new install → working transcription in <30 seconds
- [x] **4.2.8** Done

**Test checkpoint:** Delete all app data. Fresh install. Time from launch to first transcription: under 30 seconds.

### 4.3 Windows Build
- [x] **4.3.1** `cargo tauri build` for Windows (NSIS installer). Output: 7.5MB NSIS + 10.5MB MSI at src-tauri/target/release/bundle/
- [x] **4.3.2** Installer tested. Works. SmartScreen warning expected (unsigned).
- [x] **4.3.3** GPU detection: NVIDIA detected via nvidia-smi, logged at startup. Static sherpa-onnx builds are CPU-only. CUDA acceleration requires shared builds with CUDA-specific archives (deferred to future release). Provider selection wired up with fallback pattern ready.
- [ ] **4.3.4** ~~Microsoft Trusted Signing~~ - DEFERRED. Use Certum (~€30/yr) pre-launch or ship unsigned for now.
- [ ] **4.3.5** ~~Sign installer~~ - DEFERRED pre-launch
- [x] **4.3.6** Clean uninstall confirmed working.
- [ ] **4.3.7** Commit: "build: Windows installer v0.1.0"

**Test checkpoint:** Download installer from web. No scary warnings. Install, run, transcribe, uninstall. Clean.

### 4.4 macOS Build
- [ ] **4.4.1** `cargo tauri build` for macOS (DMG)
- [ ] **4.4.2** Apple Developer account setup ($99/yr)
- [ ] **4.4.3** Code sign + notarize
- [ ] **4.4.4** Test on Intel Mac and Apple Silicon Mac
- [ ] **4.4.5** Test accessibility permission flow
- [ ] **4.4.6** Verify: no Gatekeeper warning, clean install, Metal GPU acceleration on AS
- [ ] **4.4.7** Commit: "build: signed macOS DMG"

**Test checkpoint:** Download DMG. Drag to Applications. Open. No "unidentified developer" warning. Accessibility prompt is clear.

### 4.5 Linux Build
- [ ] **4.5.1** `cargo tauri build` for Linux (AppImage)
- [ ] **4.5.2** Test on Ubuntu 24.04
- [ ] **4.5.3** Document Wayland hotkey workarounds
- [ ] **4.5.4** Verify: AppImage runs, audio works, hotkey works on X11
- [ ] **4.5.5** Commit: "build: Linux AppImage"

**Test checkpoint:** Download AppImage. chmod +x. Run. Transcribe. Works on X11.

### 4.6 Auto-Update
- [ ] **4.6.1** Set up GitHub Releases as update server
- [ ] **4.6.2** Configure Tauri updater plugin
- [ ] **4.6.3** Build v0.1.0, publish as release
- [ ] **4.6.4** Build v0.1.1, publish as release
- [ ] **4.6.5** Verify: v0.1.0 detects v0.1.1, offers update, installs cleanly
- [ ] **4.6.6** Commit: "update: auto-update via GitHub Releases"

**Test checkpoint:** Install old version. It notifies of new version. Update installs without losing settings or transcripts.

### 4.7 Final Polish
- [ ] **4.7.1** Full UX review: every screen, every transition, every state
- [ ] **4.7.2** Performance check: startup time, memory usage, CPU during idle
- [ ] **4.7.3** Accessibility: keyboard navigation through all UI
- [ ] **4.7.4** README.md for the project
- [ ] **4.7.5** Test on 3+ machines (different specs, different OS)
- [ ] **4.7.6** Fix any remaining bugs
- [ ] **4.7.7** Tag v0.1.0 release
- [ ] **4.7.8** Commit: "v0.1.0: Inkwell MVP"

**Test checkpoint:** App feels premium. No jank. No crashes. Install-to-transcription under 30 seconds. You'd be proud to show it to someone.

---

## Post-MVP Backlog (v0.2+)
*Not scheduled. Pick from here after MVP ships.*

- [x] AI Polish (BYOK API keys, LLM post-processing)
- [ ] Chainable AI transforms
- [x] Per-app style overrides (detect focused app — Windows only, macOS stub)
- [x] Snippets engine (trigger phrases → text expansion, variable interpolation)
- [x] Voice commands (wake prefix, 6 defaults, risk levels, tests)
- [x] File transcription (drag + drop audio/video, symphonia decode, VAD chunking)
- [x] Export (TXT, SRT, JSON, CSV) — export.rs + export_transcripts command
- [ ] Real-time streaming transcription (Moonshine streaming mode)
- [ ] Portable mode (run from USB)
- [ ] Landing page + website (awwwards quality)
- [ ] winget / Homebrew cask / Flatpak packages
- [ ] Deploy inkwell-worker (Cloudflare Worker for free tier AI Polish proxy)

## Future (v2.0)
- [ ] MCP client integration
- [ ] Agent mode (voice → tool execution)
- [ ] Meeting mode + speaker diarization
- [ ] Calendar integration
- [ ] Smart commands (context-aware routing)
- [ ] Mobile companion app

---

*Last updated: 2026-03-29*

---

## Known Issues / Backlog Items

- **Ink sensitivity too high** (1.6.6 feedback): dial back u_amplitude or lerp speed in InkCanvas.tsx shader
- **Dev server dying** (FIXED): was PTY session timeout. Fix: launch with `Start-Process -NoNewWindow`
- **Status bar "Loading..."** (FIXED): model-loaded event fires before WebView ready. Fix: get_model_name Tauri command + invoke on mount
- **Logitech virtual mic** (FIXED): "AI Noise-cancelling Input" = cpal silence. find_preferred_device() now skips "ai noise"
- **Moonshine V1 vs V2 layout** (FIXED): engine.rs auto-detects V1 (preprocess/encode/cached_decode/uncached_decode) vs V2 (encoder/merged_decoder)
- **Parakeet load time**: 18-22 seconds first load. Consider async loading indicator in UI (status bar says "Loading Parakeet..." not "Loading...")
- **Parakeet name accuracy**: "Matthias" vs "Mattias" (one extra t). Acceptable but worth noting.
- **App process**: PID changes each run. Last known: 21512, 720MB, Parakeet loaded, Ctrl+Space active.
