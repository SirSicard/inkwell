import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import type { SnippetItem } from "../types"

export function SnippetsTab() {
  const [snippets, setSnippets] = useState<SnippetItem[]>([])
  const [trigger, setTrigger] = useState("")
  const [expansion, setExpansion] = useState("")
  const [category, setCategory] = useState("")
  const [testInput, setTestInput] = useState("")
  const [testOutput, setTestOutput] = useState("")

  useEffect(() => {
    invoke<SnippetItem[]>("get_snippets").then(setSnippets).catch(() => {})
  }, [])

  const saveAll = (updated: SnippetItem[]) => {
    setSnippets(updated)
    invoke("save_snippets", { items: updated }).catch(() => {})
  }

  const handleAdd = () => {
    if (!trigger.trim()) return
    const newSnippet: SnippetItem = {
      id: `snip-${Date.now()}`,
      trigger: trigger.trim().toLowerCase(),
      expansion: expansion,
      category: category || "General",
      enabled: true,
    }
    saveAll([...snippets, newSnippet])
    setTrigger("")
    setExpansion("")
    setCategory("")
  }

  const handleDelete = (id: string) => {
    saveAll(snippets.filter((s) => s.id !== id))
  }

  const handleToggle = (id: string) => {
    saveAll(snippets.map((s) => s.id === id ? { ...s, enabled: !s.enabled } : s))
  }

  const handleTest = () => {
    if (!testInput.trim()) return
    invoke<string>("test_snippet_expansion", { text: testInput })
      .then(setTestOutput).catch(() => {})
  }

  const categories = [...new Set(snippets.map((s) => s.category).filter(Boolean))]

  return (
    <div className="space-y-5">
      <div className="flex items-center gap-3">
        <h2 className="text-[15px] font-sans font-semibold text-text-primary">Snippets</h2>
        <span className="text-xs font-mono text-text-tertiary">{snippets.length} snippets</span>
      </div>

      <p className="text-xs text-text-tertiary">
        Say a trigger phrase and Inkwell replaces it with the full text. Supports {"{date}"}, {"{time}"}, {"{clipboard}"} variables.
      </p>

      {/* Add new snippet */}
      <div className="p-3 bg-bg-surface border border-border rounded-lg space-y-2">
        <div className="flex gap-2">
          <input
            value={trigger}
            onChange={(e) => setTrigger(e.target.value)}
            placeholder="Trigger phrase..."
            className="w-32 px-3 py-1.5 text-xs bg-bg-base border border-border rounded-md text-text-primary placeholder:text-text-tertiary focus:outline-none focus:border-border-default font-mono"
          />
          <input
            value={category}
            onChange={(e) => setCategory(e.target.value)}
            placeholder="Category"
            list="snippet-categories"
            className="w-24 px-3 py-1.5 text-xs bg-bg-base border border-border rounded-md text-text-primary placeholder:text-text-tertiary focus:outline-none focus:border-border-default"
          />
          <datalist id="snippet-categories">
            {categories.map((c) => <option key={c} value={c} />)}
          </datalist>
          <button
            onClick={handleAdd}
            disabled={!trigger.trim()}
            className="px-3 py-1.5 text-xs font-mono bg-accent text-white rounded-md hover:bg-accent/90 transition-colors disabled:opacity-40 shrink-0"
          >
            Add
          </button>
        </div>
        <textarea
          value={expansion}
          onChange={(e) => setExpansion(e.target.value)}
          placeholder='Expansion text... (e.g. "Best regards,\nMattias Herzig")'
          rows={2}
          className="w-full px-3 py-1.5 text-xs bg-bg-base border border-border rounded-md text-text-secondary placeholder:text-text-tertiary focus:outline-none focus:border-border-default resize-none"
        />
      </div>

      {/* Snippet list */}
      {snippets.length === 0 ? (
        <p className="text-sm text-text-tertiary py-8 text-center">No snippets yet. Add a trigger phrase above.</p>
      ) : (
        <div className="space-y-1.5">
          {snippets.map((s) => (
            <div
              key={s.id}
              className={`group flex items-center gap-3 p-2.5 bg-bg-surface border border-border rounded-lg transition-colors ${
                !s.enabled ? "opacity-40" : ""
              }`}
            >
              <button
                onClick={() => handleToggle(s.id)}
                className={`w-2 h-2 rounded-full shrink-0 transition-colors ${
                  s.enabled ? "bg-green-400" : "bg-border"
                }`}
                title={s.enabled ? "Enabled" : "Disabled"}
              />
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <span className="text-xs font-mono text-accent">{s.trigger}</span>
                  {s.category && (
                    <span className="text-[10px] px-1 py-0.5 rounded bg-bg-base text-text-tertiary">{s.category}</span>
                  )}
                </div>
                <p className="text-xs text-text-tertiary truncate mt-0.5">{s.expansion}</p>
              </div>
              <button
                onClick={() => handleDelete(s.id)}
                className="px-2 py-1 text-xs text-text-tertiary hover:text-red-400 transition-colors opacity-0 group-hover:opacity-100"
              >
                Del
              </button>
            </div>
          ))}
        </div>
      )}

      {/* Test panel */}
      <div className="space-y-2 pt-2 border-t border-border/50">
        <p className="text-[11px] font-medium text-text-tertiary tracking-wide">Test</p>
        <div className="flex gap-2">
          <input
            value={testInput}
            onChange={(e) => setTestInput(e.target.value)}
            placeholder="Type text with a trigger phrase..."
            className="flex-1 px-3 py-1.5 text-xs bg-bg-surface border border-border rounded-md text-text-primary placeholder:text-text-tertiary focus:outline-none focus:border-border-default"
          />
          <button
            onClick={handleTest}
            disabled={!testInput.trim()}
            className="px-3 py-1.5 text-xs font-mono bg-bg-surface border border-border text-text-secondary hover:text-text-primary rounded-md transition-colors disabled:opacity-40"
          >
            Expand
          </button>
        </div>
        {testOutput && (
          <div className="p-2.5 bg-bg-surface border border-border rounded-lg">
            <p className="text-xs text-text-primary whitespace-pre-wrap">{testOutput}</p>
          </div>
        )}
      </div>
    </div>
  )
}
