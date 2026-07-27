import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { InkButton } from "../components/ui"
import { toast } from "../state/toasts"
import type { Mode, ModeStore } from "../types"

/**
 * Modes replace three concepts that used to decide independently how a
 * dictation is written: the global text style, the per-app style rules, and the
 * global polish prompt. A mode holds all of them together, so "formal, polished
 * for email, only in Outlook" is expressible for the first time.
 *
 * The list order is the precedence rule: the first mode whose app list matches
 * the frontmost application wins. That is stated in the UI rather than left for
 * the user to discover from behaviour.
 */

const STYLES = [
  { id: "formal", label: "Formal", hint: "Capitals and punctuation" },
  { id: "casual", label: "Casual", hint: "Capitals, lighter punctuation" },
  { id: "relaxed", label: "Relaxed", hint: "Lowercase, minimal punctuation" },
] as const

export function ModesTab() {
  const [store, setStore] = useState<ModeStore | null>(null)
  const [saving, setSaving] = useState(false)

  const load = useCallback(() => {
    invoke<ModeStore>("get_modes")
      .then(setStore)
      .catch((e) => toast(`Could not load modes: ${e}`, "warning"))
  }, [])

  useEffect(() => { load() }, [load])

  const persist = async (next: ModeStore) => {
    setStore(next)
    setSaving(true)
    try {
      await invoke("save_modes", { store: next })
    } catch (e) {
      toast(`Could not save modes: ${e}`)
      load()
    }
    setSaving(false)
  }

  if (!store) return <p className="text-body text-text-tertiary">Loading modes...</p>

  const update = (id: string, patch: Partial<Mode>) =>
    persist({ ...store, modes: store.modes.map((m) => (m.id === id ? { ...m, ...patch } : m)) })

  const addMode = () => {
    const id = `mode-${Date.now()}`
    persist({
      ...store,
      modes: [
        ...store.modes,
        {
          id,
          name: "New mode",
          style: "formal",
          model: "",
          polish_prompt: "",
          polish_enabled: false,
          apps: [],
          remove_fillers: true,
        },
      ],
    })
  }

  const removeMode = (id: string) => {
    if (id === store.default_id) {
      toast("The default mode cannot be removed. Make another mode the default first.", "warning")
      return
    }
    persist({ ...store, modes: store.modes.filter((m) => m.id !== id) })
  }

  const move = (id: string, delta: number) => {
    const i = store.modes.findIndex((m) => m.id === id)
    const j = i + delta
    if (i < 0 || j < 0 || j >= store.modes.length) return
    const modes = [...store.modes]
    ;[modes[i], modes[j]] = [modes[j], modes[i]]
    persist({ ...store, modes })
  }

  const addCurrentApp = async (id: string) => {
    try {
      const app = await invoke<string | null>("get_foreground_app")
      if (!app) {
        toast("Could not identify the frontmost app.", "warning")
        return
      }
      const mode = store.modes.find((m) => m.id === id)
      if (!mode) return
      if (mode.apps.some((a) => a.toLowerCase() === app.toLowerCase())) {
        toast(`${app} is already in this mode.`, "info")
        return
      }
      update(id, { apps: [...mode.apps, app] })
      toast(`Added ${app}.`, "info")
    } catch (e) {
      toast(`Could not read the frontmost app: ${e}`)
    }
  }

  return (
    <div className="space-y-4">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h2 className="text-heading font-sans font-semibold text-text-primary">Modes</h2>
          <p className="text-body text-text-tertiary mt-0.5">
            A mode decides how a dictation is written. The first mode whose apps match what
            you are typing into wins; otherwise the default is used.
          </p>
        </div>
        <InkButton onClick={addMode} disabled={saving}>Add mode</InkButton>
      </div>

      <div className="space-y-2.5">
        {store.modes.map((m, i) => {
          const isDefault = m.id === store.default_id
          return (
            <div
              key={m.id}
              className={`rounded-lg border p-3.5 space-y-3 transition-colors ${
                isDefault ? "bg-accent/[0.05] border-accent/25" : "bg-bg-surface border-border"
              }`}
            >
              <div className="flex items-center gap-2">
                <input
                  value={m.name}
                  onChange={(e) => update(m.id, { name: e.target.value })}
                  aria-label="Mode name"
                  className="flex-1 px-2.5 py-1.5 text-body font-medium bg-bg-base border border-border rounded-md text-text-primary focus:outline-none focus:border-border-default"
                />
                {isDefault ? (
                  <span className="text-meta font-mono text-accent px-2">default</span>
                ) : (
                  <button
                    onClick={() => persist({ ...store, default_id: m.id })}
                    className="text-body text-text-tertiary hover:text-text-primary px-2 transition-colors"
                  >
                    Make default
                  </button>
                )}
                <button
                  onClick={() => move(m.id, -1)}
                  disabled={i === 0}
                  aria-label="Move mode up"
                  className="px-1.5 text-body text-text-tertiary hover:text-text-primary disabled:opacity-30 transition-colors"
                >
                  ↑
                </button>
                <button
                  onClick={() => move(m.id, 1)}
                  disabled={i === store.modes.length - 1}
                  aria-label="Move mode down"
                  className="px-1.5 text-body text-text-tertiary hover:text-text-primary disabled:opacity-30 transition-colors"
                >
                  ↓
                </button>
                <button
                  onClick={() => removeMode(m.id)}
                  aria-label="Delete mode"
                  className="px-1.5 text-body text-text-tertiary hover:text-red-400 transition-colors"
                >
                  Del
                </button>
              </div>

              <div className="flex flex-wrap gap-1.5">
                {STYLES.map((s) => (
                  <button
                    key={s.id}
                    onClick={() => update(m.id, { style: s.id })}
                    title={s.hint}
                    className={`px-2.5 py-1 text-body rounded-md border transition-colors ${
                      m.style === s.id
                        ? "bg-accent/[0.10] border-accent/30 text-text-primary"
                        : "bg-bg-base border-border text-text-secondary hover:text-text-primary"
                    }`}
                  >
                    {s.label}
                  </button>
                ))}
                <label className="flex items-center gap-1.5 text-body text-text-secondary ml-1">
                  <input
                    type="checkbox"
                    checked={m.remove_fillers}
                    onChange={(e) => update(m.id, { remove_fillers: e.target.checked })}
                  />
                  Clean up speech
                </label>
                <label className="flex items-center gap-1.5 text-body text-text-secondary ml-1">
                  <input
                    type="checkbox"
                    checked={m.polish_enabled}
                    onChange={(e) => update(m.id, { polish_enabled: e.target.checked })}
                  />
                  AI polish
                </label>
              </div>

              {m.polish_enabled && (
                <input
                  value={m.polish_prompt}
                  onChange={(e) => update(m.id, { polish_prompt: e.target.value })}
                  placeholder="Polish prompt for this mode (blank uses the one in AI settings)"
                  className="w-full px-2.5 py-1.5 text-body bg-bg-base border border-border rounded-md text-text-primary placeholder:text-text-tertiary focus:outline-none focus:border-border-default"
                />
              )}

              <div className="space-y-1.5">
                <div className="flex items-center justify-between gap-2">
                  <p className="text-meta font-mono uppercase tracking-wider text-text-tertiary">
                    {isDefault ? "Used when nothing else matches" : "Active in these apps"}
                  </p>
                  {!isDefault && (
                    <button
                      onClick={() => addCurrentApp(m.id)}
                      className="text-body text-text-tertiary hover:text-text-primary transition-colors"
                    >
                      Add current app
                    </button>
                  )}
                </div>
                {!isDefault && (
                  <div className="flex flex-wrap gap-1.5">
                    {m.apps.map((a) => (
                      <span
                        key={a}
                        className="inline-flex items-center gap-1.5 px-2 py-1 text-body bg-bg-base border border-border rounded-md text-text-secondary"
                      >
                        {a}
                        <button
                          onClick={() => update(m.id, { apps: m.apps.filter((x) => x !== a) })}
                          aria-label={`Remove ${a}`}
                          className="text-text-tertiary hover:text-red-400"
                        >
                          ×
                        </button>
                      </span>
                    ))}
                    {m.apps.length === 0 && (
                      <span className="text-body text-text-tertiary">
                        No apps yet, so this mode never activates on its own.
                      </span>
                    )}
                  </div>
                )}
              </div>
            </div>
          )
        })}
      </div>
    </div>
  )
}
