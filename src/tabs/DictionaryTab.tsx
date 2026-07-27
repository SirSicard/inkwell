import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "../state/toasts"
import type { DictEntry } from "../types"

export function DictionaryTab() {
  const [entries, setEntries] = useState<DictEntry[]>([])
  const [newFind, setNewFind] = useState("")
  const [newReplace, setNewReplace] = useState("")

  useEffect(() => {
    invoke<DictEntry[]>("get_dictionary").then(setEntries).catch((e) => toast(`Could not load dictionary: ${e}`, "warning"))
  }, [])

  const save = (updated: DictEntry[]) => {
    setEntries(updated)
    invoke("set_dictionary", { entries: updated }).catch((e) => toast(`Could not save dictionary: ${e}`))
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

  // CSV import. superwhisper ships the same dictionary feature with CSV import
  // and calls it custom vocabulary; the difference was purely the affordance.
  // Accepts "find,replace" per line, tolerates a header row and quoted fields,
  // and merges rather than replacing so an import cannot wipe existing entries.
  const handleImport = (file: File) => {
    const reader = new FileReader()
    reader.onerror = () => toast("Could not read that file")
    reader.onload = () => {
      const text = String(reader.result ?? "")
      const rows = text
        .split(/\r?\n/)
        .map((line) => line.trim())
        .filter(Boolean)
        .map((line) => {
          // Split on the first comma only, so a replacement may contain commas.
          const i = line.indexOf(",")
          if (i < 0) return null
          const strip = (v: string) => v.trim().replace(/^"(.*)"$/s, "$1").trim()
          return { find: strip(line.slice(0, i)), replace: strip(line.slice(i + 1)) }
        })
        .filter((r): r is DictEntry => !!r && r.find.length > 0)

      if (rows.length === 0) {
        toast("No usable rows found. Expected one find,replace pair per line.", "warning")
        return
      }

      // Drop a header row only if it looks like one, never by position alone.
      const first = rows[0].find.toLowerCase()
      const body = first === "find" || first === "word" || first === "from" ? rows.slice(1) : rows

      const byFind = new Map(entries.map((e) => [e.find.toLowerCase(), e]))
      let added = 0
      let updated = 0
      for (const row of body) {
        if (byFind.has(row.find.toLowerCase())) updated++
        else added++
        byFind.set(row.find.toLowerCase(), row)
      }
      save([...byFind.values()])
      toast(`Imported ${added} new, updated ${updated}.`, "info")
    }
    reader.readAsText(file)
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-3">
        <h2 className="text-heading font-sans font-semibold text-text-primary">Dictionary</h2>
        <span className="text-xs font-mono text-text-tertiary">{entries.length} entries</span>
      </div>
      <div className="flex items-start justify-between gap-3">
        <p className="text-xs text-text-tertiary">
          Auto-correct words after transcription. Case-insensitive matching, word boundaries only.
        </p>
        <label className="shrink-0 px-3 py-1.5 text-xs font-mono rounded-md border border-border text-text-secondary hover:text-text-primary hover:border-border-default transition-colors cursor-pointer">
          Import CSV
          <input
            type="file"
            accept=".csv,text/csv,text/plain"
            className="hidden"
            onChange={(e) => {
              const f = e.target.files?.[0]
              if (f) handleImport(f)
              // Reset so choosing the same file twice still fires onChange.
              e.target.value = ""
            }}
          />
        </label>
      </div>

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
