import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import type { DictEntry } from "../types"

export function DictionaryTab() {
  const [entries, setEntries] = useState<DictEntry[]>([])
  const [newFind, setNewFind] = useState("")
  const [newReplace, setNewReplace] = useState("")

  useEffect(() => {
    invoke<DictEntry[]>("get_dictionary").then(setEntries).catch(() => {})
  }, [])

  const save = (updated: DictEntry[]) => {
    setEntries(updated)
    invoke("set_dictionary", { entries: updated }).catch((e) => console.error("set_dictionary failed:", e))
  }

  const handleAdd = () => {
    if (!newFind.trim()) return
    save([...entries, { find: newFind.trim(), replace: newReplace.trim() }])
    setNewFind("")
    setNewReplace("")
  }

  const handleRemove = (index: number) => {
    save(entries.filter((_, i) => i !== index))
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-3">
        <h2 className="text-[15px] font-sans font-semibold text-text-primary">Dictionary</h2>
        <span className="text-xs font-mono text-text-tertiary">{entries.length} entries</span>
      </div>
      <p className="text-xs text-text-tertiary">
        Auto-correct words after transcription. Case-insensitive matching, word boundaries only.
      </p>

      <div className="flex gap-2 items-end">
        <div className="flex-1">
          <label className="text-xs text-text-tertiary mb-1 block">Find</label>
          <input
            type="text"
            value={newFind}
            onChange={(e) => setNewFind(e.target.value)}
            placeholder="e.g. matthias"
            className="w-full px-3 py-2 text-sm bg-bg-surface border border-border rounded-lg text-text-primary placeholder:text-text-tertiary focus:outline-none focus:border-border-default"
            onKeyDown={(e) => e.key === "Enter" && handleAdd()}
          />
        </div>
        <div className="flex-1">
          <label className="text-xs text-text-tertiary mb-1 block">Replace with</label>
          <input
            type="text"
            value={newReplace}
            onChange={(e) => setNewReplace(e.target.value)}
            placeholder="e.g. Mattias"
            className="w-full px-3 py-2 text-sm bg-bg-surface border border-border rounded-lg text-text-primary placeholder:text-text-tertiary focus:outline-none focus:border-border-default"
            onKeyDown={(e) => e.key === "Enter" && handleAdd()}
          />
        </div>
        <button
          onClick={handleAdd}
          className="px-4 py-2 text-sm bg-bg-surface border border-border rounded-lg text-text-secondary hover:text-text-primary hover:border-border-default transition-colors shrink-0"
        >
          Add
        </button>
      </div>

      {entries.length === 0 ? (
        <p className="text-sm text-text-tertiary py-8 text-center">No dictionary entries yet. Add one above.</p>
      ) : (
        <div className="space-y-1.5">
          {entries.map((entry, i) => (
            <div
              key={i}
              className="group flex items-center gap-3 px-3 py-2 bg-bg-surface border border-border rounded-lg"
            >
              <span className="text-sm text-text-tertiary font-mono flex-1">{entry.find}</span>
              <span className="text-xs text-text-tertiary">→</span>
              <span className="text-sm text-text-primary font-mono flex-1">{entry.replace}</span>
              <button
                onClick={() => handleRemove(i)}
                className="text-xs text-text-tertiary hover:text-red-400 transition-colors opacity-0 group-hover:opacity-100"
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
