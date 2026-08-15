/**
 * Browser-only Tauri shim for visual passes.
 *
 * The real app can only be screenshotted by launching it, which collides with
 * the single-instance lock of whatever Inkwell the machine is already running,
 * and once cost a near-publication of real transcripts. Loading the Vite dev
 * server in an ordinary browser with this shim renders the identical frontend
 * against invented fixtures instead, where DOM tools can drive it.
 *
 * Never imported in a Tauri build: main.tsx only loads it when the Tauri
 * runtime is absent, and the guard is on import.meta.env.DEV as well, so a
 * production bundle cannot contain it.
 */

const today = new Date()
const day = (offset: number) => {
  const d = new Date(today)
  d.setDate(d.getDate() - offset)
  return d.toISOString().slice(0, 10)
}

const settings = {
  style: "formal", model: "parakeet-v2", hotkey: "shift+super+space",
  edit_hotkey: "super+shift+e", recording_mode: "ptt", start_on_boot: false,
  show_overlay: true, overlay_position: "bottom-center", theme: "dark",
  advanced_mode: true, mic_device: "auto", vad_threshold: 0.5,
  polish_enabled: false, polish_prompt: "", sound_dictation: false,
  remove_fillers: true, debug_save_audio: false,
  mic_idle_release_mins: 3, append_space: true,
}

const stats = {
  total_count: 39, total_words: 412, total_speaking_ms: 158_000,
  days_active: 10, streak_days: 3,
  best_day: [day(5), 9],
  recent_days: [
    [day(13), 4, 41], [day(12), 0, 0], [day(11), 2, 22], [day(10), 3, 30],
    [day(9), 0, 0], [day(8), 1, 12], [day(7), 5, 51], [day(6), 0, 0],
    [day(5), 9, 96], [day(4), 2, 20], [day(3), 0, 0], [day(2), 3, 33],
    [day(1), 6, 62], [day(0), 4, 45],
  ],
  per_model: [["Parakeet V2", 20], ["Qwen3 ASR", 13], ["SenseVoice", 6]],
}

const transcripts = [
  { id: 1, text: "Let's ship the release notes today and pick this up again on Monday.", raw_text: "", style: "formal", model: "Qwen3 ASR", audio_duration_ms: 4200, created_at: `${day(0)} 09:12:00` },
  { id: 2, text: "Can you take a look at the pull request when you get a chance? No rush.", raw_text: "", style: "formal", model: "Qwen3 ASR", audio_duration_ms: 4900, created_at: `${day(0)} 09:08:00` },
  { id: 3, text: "thanks, that fixed it. merging now", raw_text: "", style: "relaxed", model: "Parakeet V2", audio_duration_ms: 2600, created_at: `${day(1)} 16:47:00` },
]

const fixtures: Record<string, unknown> = {
  get_settings: settings,
  get_stats: stats,
  get_transcripts: transcripts,
  search_transcripts: transcripts,
  get_model_name: "Parakeet V2",
  get_input_devices: [{ id: "MacBook Pro Microphone", name: "MacBook Pro Microphone" }],
  check_first_run: false,
  check_accessibility_permission: true,
  get_dictionary: [],
  get_pinned_mode: null,
  list_models: [],
  get_snippets: [],
  get_modes: { modes: [], default_id: "default" },
  get_voice_commands: { commands: [], enabled: false },
  "plugin:app|version": "0.2.8-dev",
  "plugin:event|listen": 1,
  "plugin:event|unlisten": null,
  "plugin:updater|check": null,
}

import { MotionGlobalConfig } from "framer-motion"

// An occluded browser throttles requestAnimationFrame to nothing, which parks
// AnimatePresence's mode="wait" on an exit animation forever: the tab state
// flips (aria-selected moves) while the old panel never unmounts. Animations
// are cosmetic in a visual pass, so skip them wholesale rather than trying to
// drive a frameloop the browser refuses to tick.
MotionGlobalConfig.skipAnimations = true

// A hidden or occluded browser throttles requestAnimationFrame to nothing,
// which leaves AnimatePresence's mode="wait" waiting forever on an exit
// animation and freezes tab switching. Timers still fire (clamped), so a
// timer-driven rAF keeps time-based animations completing while the page is
// not frontmost. Visual-pass harness behaviour only; never ships.
window.requestAnimationFrame = (cb: FrameRequestCallback) =>
  window.setTimeout(() => cb(performance.now()), 16)
window.cancelAnimationFrame = (id: number) => window.clearTimeout(id)

;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
  transformCallback: (() => {
    let n = 1
    return () => n++
  })(),
  metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  // StrictMode mounts effects twice; the event API's unlisten path calls this.
  unregisterListener: () => {},
  invoke: (cmd: string) =>
    cmd in fixtures
      ? Promise.resolve(fixtures[cmd])
      : Promise.reject(`devmock: no fixture for ${cmd}`),
}

export {}
