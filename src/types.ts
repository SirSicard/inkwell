export interface Settings {
  style: string
  model: string
  hotkey: string
  recording_mode: string
  start_on_boot: boolean
  show_overlay: boolean
  advanced_mode: boolean
  mic_device: string
  vad_threshold: number
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
export const advancedTabs = ["Dashboard", "General", "Audio", "Models", "AI", "Agent", "Snippets", "App Styles", "Dictionary", "Files", "Commands", "About"] as const
export type Tab = (typeof advancedTabs)[number]
