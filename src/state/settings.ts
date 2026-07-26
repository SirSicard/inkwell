import { create } from "zustand"
import { invoke } from "@tauri-apps/api/core"
import type { Settings } from "../types"
import { toast } from "./toasts"

/// One owner for settings.
///
/// Every tab used to call get_settings on mount and write fields back
/// independently, so twelve components each held their own copy with no cache
/// coherence. mic_device, for instance, was editable from both onboarding and
/// the Audio tab through separate code paths, and neither knew when the other
/// changed it. Failures were swallowed by empty catch blocks, so a rejected
/// write left the UI showing a value the backend had never accepted.
///
/// This store hydrates once, applies changes optimistically so the UI stays
/// responsive, and rolls back with a visible toast when the backend refuses.

interface SettingsState {
  settings: Settings | null
  loaded: boolean
  load: () => Promise<void>
  /// Update one key. Optimistic; reverts and reports if the backend rejects it.
  set: <K extends keyof Settings>(key: K, value: Settings[K]) => Promise<void>
}

export const useSettings = create<SettingsState>((set, get) => ({
  settings: null,
  loaded: false,

  load: async () => {
    try {
      const settings = await invoke<Settings>("get_settings")
      set({ settings, loaded: true })
    } catch (e) {
      // Not fatal, since the app still runs on backend defaults, but the user
      // should know the panel they are looking at may be stale.
      toast(`Could not load settings: ${e}`)
      set({ loaded: true })
    }
  },

  set: async (key, value) => {
    const current = get().settings
    if (!current) return
    const previous = current[key]
    if (previous === value) return

    set({ settings: { ...current, [key]: value } })
    try {
      // The backend takes every setting as a string and parses per key.
      await invoke("update_settings", { key, value: String(value) })
    } catch (e) {
      const latest = get().settings
      if (latest) set({ settings: { ...latest, [key]: previous } })
      // Real refusals reach the user now. update_settings rejects a mic switch
      // during recording, for one, which used to vanish into an empty catch.
      toast(`${String(key).replace(/_/g, " ")}: ${e}`)
    }
  },
}))
