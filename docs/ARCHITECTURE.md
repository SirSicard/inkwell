# Architecture (target state)

**This describes where the code is going, not where it is.** The current `src-tauri` violates several of these rules in ways listed at the bottom. Adapted from the inkwell-v2 rebuild spec, which was a good design document attached to a codebase that never worked. The rules survive, the rewrite does not.

Use this when deciding where a new piece of code belongs, and when reviewing a change that moves logic across a boundary.

---

## Layers

| Layer | Contains | Must not |
|---|---|---|
| **shell** | Tauri commands, event emission, window and tray wiring, plugin setup | Contain product logic, branch on business rules, or own state |
| **app** | Services and use-case orchestration. The dictation service, the file-transcription service, the model service | Know about Tauri types, HWNDs, or HTTP clients |
| **domain** | Stable types, the staged pipeline definition, pure text transforms (style, dictionary, snippets, voice commands) | Do I/O of any kind |
| **infra** | Concrete adapters: sherpa-onnx recognizer, SQLite history, HTTP LLM providers, keyring, file downloads | Be imported by domain |
| **platform** | OS boundaries: audio capture, global hotkeys, synthetic paste, focused-app detection, permission checks | Leak platform-specific types upward |
| **contracts** | Typed command, event and job payloads shared with the frontend | Be bypassed by ad hoc `serde_json::Value` payloads |

Dependency direction is one way: shell -> app -> domain, with infra and platform plugged in behind traits that app and domain define. Nothing lower reaches up.

## The rules

1. **No product logic in Tauri commands.** A command validates its input, calls one service method, and maps the result. If a command body contains a pipeline, it is in the wrong place.
2. **The offline loop must work with every remote integration disabled.** Dictate with no network, no polish key, no update server. If that path breaks, nothing else matters.
3. **No plaintext secret fallback, ever.** Keys and tokens go to the OS keyring. If the keyring fails, the feature fails loudly. "Also save a copy to settings.json as backup" is how v1 leaked the agent token.
4. **No raw user audio or text written outside the user's data directory.** No debug WAVs in temp, no transcripts in logs. Anything of that kind is a build-time flag that is off by default, not a runtime path.
5. **One pipeline, shared stages.** Dictation and file transcription differ in their source and their sink, not in their middle. Do not duplicate the chain to add a variant.
6. **Typed contracts are the source of truth** for anything crossing the webview boundary. Stringly-typed payloads drift silently, which is exactly how a settings tab can ship without ever persisting anything.
7. **Long work never runs on the UI thread.** Model loading (hundreds of MB), file transcription and inference go through `spawn_blocking` or a worker. A frozen window during a model switch is a bug, not a wait.
8. **Coarse state, not a mutex per field.** Group related state behind a small number of services. Every `lock().unwrap()` is a panic waiting for a poisoned mutex to cascade.
9. **Registries are data, not control flow.** One table describes each model (id, files, URLs, size, engine kind). Adding a model must be a row, not six match arms across three files.
10. **Platform differences are isolated and honest.** A macOS stub that returns `None` and silently disables a feature is worse than an explicit "unsupported on this platform" the UI can show.

## The canonical pipeline

Every dictation and every file transcription runs the same staged chain, differing only at the ends.

1. capture audio (mic stream, or decode a file)
2. normalize and resample to 16 kHz mono
3. VAD segmentation (optional, skipped if the model is unavailable)
4. transcribe
5. detect voice commands
6. apply style transform
7. apply dictionary transform
8. expand snippets
9. AI polish (optional, BYOK, off by default)
10. persist transcript
11. emit the output action (clipboard plus paste, or a file)

Stages 5 through 8 are pure functions over strings and are the part currently covered by the 60 tests in `src-tauri/tests/pipeline_tests.rs`. Keep them pure. That is why they are testable and why they have not caused a bug.

**Live preview is a branch off stage 1 that never rejoins.** `streaming.rs` taps the same capture buffer, decodes it with its own model on its own thread, and draws the result on the overlay. It stops there: it never reaches stage 4, so nothing it produces is styled, stored, or pasted, and the chain above is unchanged whether it is running or not. Read it as a second consumer of the audio, not as a second pipeline. The one rule it adds is that it must never be able to affect the output of the real one, which is why the two share no state beyond the buffer they both read.

## Testing expectations

- Pipeline stages are tested independently, with no Tauri and no AppState in scope.
- The 60 existing pipeline tests are the regression floor. A change that makes one obsolete deletes that test and says so in the commit. A change that makes one fail does not get its assertion weakened.
- Adapters (recognizer, history, providers) are exercised behind their traits so the services can be tested without a model on disk.

## Where the current code violates this

Honest list, from the July 2026 review. Line references were accurate at commit `d2f119e`.

| Rule | Violation |
|---|---|
| 1, 5 | The entire dictation pipeline is inline in the global-shortcut handler closure in `pipeline.rs`, and constructs a fresh tokio runtime plus a nested thread per LLM call |
| 1, 9 | `commands.rs` is a 913-line god file that embeds the model registry as roughly six separate hardcoded match statements, plus more in `engine.rs` and `setup.rs`. Six places to touch per model. |
| 8 | `AppState` holds 22 mutex fields with 123 `lock().unwrap()` call sites |
| 7 | `switch_model` loads a ~700 MB ONNX encoder synchronously on the main thread. `transcribe_file` holds the engine mutex for the whole file and blocks hotkey dictation. |
| 3 | Fixed in the rehaul: the agent token was written to `settings.json` in plaintext even when the keyring write succeeded |
| 4 | Fixed in the rehaul: raw audio of every dictation was written to a temp WAV in the production path |
| 10 | `appdetect.rs` is Win32 only and its macOS stub returns `None`, so per-app styles silently do nothing on the primary platform |
| 2 | The offline loop is intact, but VAD silently degrades because nothing downloads `silero_vad.onnx` |
| 6 | The removed Agent tab shipped with a mismatched invoke shape and no backend handlers, so nothing it saved ever persisted. Untyped payloads made that invisible. |
| shell/app | `setup.rs` is a 284-line setup function doing state init, model loading, migration and window wiring in one body |

Also outstanding, not layer violations but same review: CSP is disabled (`csp: null`) with broad clipboard and filesystem capabilities granted to both windows, chunk-overlap concatenation has no dedup so words repeat at 15s boundaries, and paste clobbers the user's clipboard without restoring it.

## Approach

Strangle, do not rewrite. v1 is a working product whose debt sits in four files. Extract one service at a time behind the existing tests, starting with the dictation pipeline. A rewrite was already attempted and produced 1,088 lines, zero tests and a fake transcription path. See [rehaul-analysis-2026-07-24.md](rehaul-analysis-2026-07-24.md).
