/** Mirrors the Rust `Settings` struct in src-tauri/src/settings.rs; keep in sync. */
export interface Settings {
  style: string
  model: string
  hotkey: string
  recording_mode: string
  start_on_boot: boolean
  show_overlay: boolean
  overlay_position: string
  advanced_mode: boolean
  mic_device: string
  vad_threshold: number
  polish_enabled: boolean
  polish_prompt: string
  sound_dictation: boolean
  debug_save_audio: boolean
}

export interface Toast {
  id: number
  message: string
  type: "error" | "warning" | "info"
}

export interface Transcript {
  id: number
  text: string
  raw_text: string
  style: string
  model: string
  audio_duration_ms: number
  created_at: string
}

export interface UpdateInfo {
  version: string
  notes: string
  date: string
}

export interface DeviceInfo {
  id: string
  name: string
}

export interface DictEntry {
  find: string
  replace: string
}

export interface AppStyleRule {
  process_name: string
  style: string
}

export interface SnippetItem {
  id: string
  trigger: string
  expansion: string
  category: string
  enabled: boolean
}

export interface VoiceCommandItem {
  id: string
  triggers: string[]
  action: { type: string; style?: string; model?: string; url?: string; path?: string; text?: string }
  enabled: boolean
}

export interface VoiceCommandStoreData {
  enabled: boolean
  wake_prefix: string
  commands: VoiceCommandItem[]
}

export interface FileTranscribeResult {
  filename: string
  duration_s: number
  text: string
  raw_text: string
  segments: { start_ms: number; end_ms: number; text: string }[]
}

export const basicTabs = ["Dashboard", "General", "About"] as const
export const advancedTabs = ["Dashboard", "General", "Audio", "Models", "AI", "Snippets", "App Styles", "Dictionary", "Files", "Commands", "About"] as const
export type Tab = (typeof advancedTabs)[number]

/**
 * Sidebar grouping.
 *
 * Twelve flat tabs across the top overflowed into a three-dot menu at the
 * default 800px window, hiding roughly half the app, and basic/advanced swapped
 * the whole tab set so nothing stayed where the user left it. Grouping them
 * down the side removes the overflow entirely and keeps every item in a fixed
 * position; advanced mode now reveals extra items in place rather than
 * replacing the list.
 */
export const tabGroups: readonly { label: string; tabs: readonly Tab[] }[] = [
  { label: "History", tabs: ["Dashboard", "Files"] },
  { label: "Dictation", tabs: ["General", "Audio", "Models"] },
  { label: "Intelligence", tabs: ["AI", "Snippets", "Dictionary", "Commands", "App Styles"] },
  { label: "System", tabs: ["About"] },
]
