import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { GlassToggle } from "../components/ui"
import type { AppStyleRule } from "../types"

export function AppStylesTab() {
  const [enabled, setEnabled] = useState(false)
  const [rules, setRules] = useState<AppStyleRule[]>([])
  const [newProcess, setNewProcess] = useState("")
  const [newStyle, setNewStyle] = useState("formal")

  useEffect(() => {
    invoke<{ enabled: boolean; rules: AppStyleRule[] }>("get_app_styles").then((data) => {
      setEnabled(data.enabled)
      setRules(data.rules)
    }).catch(() => {})
  }, [])

  const saveAll = (en: boolean, r: AppStyleRule[]) => {
    setEnabled(en)
    setRules(r)
    invoke("save_app_styles", { rules: { enabled: en, rules: r } }).catch(() => {})
  }

  const handleAdd = () => {
    if (!newProcess.trim()) return
    const updated = [...rules, { process_name: newProcess.trim().toLowerCase(), style: newStyle }]
    saveAll(enabled, updated)
    setNewProcess("")
  }

  const handleDelete = (idx: number) => {
    const updated = rules.filter((_, i) => i !== idx)
    saveAll(enabled, updated)
  }

  const handleStyleChange = (idx: number, style: string) => {
    const updated = rules.map((r, i) => i === idx ? { ...r, style } : r)
    saveAll(enabled, updated)
  }

  return (
    <div className="space-y-5">
      <div className="flex items-center gap-3">
        <h2 className="text-[15px] font-sans font-semibold text-text-primary">Per-App Styles</h2>
      </div>

      <p className="text-xs text-text-tertiary">
        Automatically switch style based on which app you're dictating into. When disabled, the global style setting is used.
      </p>

      <div className="flex items-center justify-between p-3 bg-bg-surface border border-border rounded-lg">
        <div>
          <p className="text-sm text-text-primary">Enable per-app styles</p>
          <p className="text-xs text-text-tertiary mt-0.5">Override the global style based on the focused application</p>
        </div>
        <GlassToggle checked={enabled} onChange={(v) => saveAll(v, rules)} />
      </div>

      {/* Add rule */}
      <div className="flex gap-2 items-center">
        <input
          value={newProcess}
          onChange={(e) => setNewProcess(e.target.value)}
          placeholder="Process name (e.g. slack.exe)"
          className="flex-1 px-3 py-1.5 text-xs bg-bg-surface border border-border rounded-md text-text-primary placeholder:text-text-tertiary focus:outline-none focus:border-border-default font-mono"
        />
        <select
          value={newStyle}
          onChange={(e) => setNewStyle(e.target.value)}
          className="px-2 py-1.5 text-xs bg-bg-surface border border-border rounded-md text-text-primary focus:outline-none font-mono"
        >
          <option value="formal">Formal</option>
          <option value="casual">Casual</option>
          <option value="relaxed">Relaxed</option>
        </select>
        <button
          onClick={handleAdd}
          disabled={!newProcess.trim()}
          className="px-3 py-1.5 text-xs font-mono bg-accent text-white rounded-md hover:bg-accent/90 transition-colors disabled:opacity-40 shrink-0"
        >
          Add
        </button>
      </div>

      {/* Rules list */}
      {rules.length === 0 ? (
        <p className="text-sm text-text-tertiary py-8 text-center">No rules yet. Add a process name above.</p>
      ) : (
        <div className="space-y-1.5">
          {rules.map((r, i) => (
            <div key={i} className="group flex items-center gap-3 p-2.5 bg-bg-surface border border-border rounded-lg">
              <span className="text-xs font-mono text-accent flex-1">{r.process_name}</span>
              <select
                value={r.style}
                onChange={(e) => handleStyleChange(i, e.target.value)}
                className="px-2 py-1 text-xs bg-bg-base border border-border rounded-md text-text-primary focus:outline-none font-mono"
              >
                <option value="formal">Formal</option>
                <option value="casual">Casual</option>
                <option value="relaxed">Relaxed</option>
              </select>
              <button
                onClick={() => handleDelete(i)}
                className="px-2 py-1 text-xs text-text-tertiary hover:text-red-400 transition-colors opacity-0 group-hover:opacity-100"
              >
                Del
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
