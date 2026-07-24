import { useState, useEffect } from "react"
import { motion, AnimatePresence } from "framer-motion"
import { listen } from "@tauri-apps/api/event"
import { invoke } from "@tauri-apps/api/core"
import type { Transcript } from "../types"

function ExportMenu({
  exportFormat, setExportFormat, onSave, onCopy, exporting,
}: {
  exportFormat: string
  setExportFormat: (f: "txt" | "srt" | "json" | "csv") => void
  onSave: () => void
  onCopy: () => void
  exporting: boolean
}) {
  const [open, setOpen] = useState(false)

  return (
    <div className="ml-auto relative">
      <button
        onClick={() => setOpen(!open)}
        className={`flex items-center justify-center w-7 h-7 rounded-md border transition-colors ${
          open ? "border-accent/30 text-text-primary bg-accent/10" : "border-border text-text-tertiary hover:text-text-secondary"
        }`}
        title="Export"
      >
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none">
          <circle cx="8" cy="3" r="1.2" fill="currentColor" />
          <circle cx="8" cy="8" r="1.2" fill="currentColor" />
          <circle cx="8" cy="13" r="1.2" fill="currentColor" />
        </svg>
      </button>
      <AnimatePresence>
        {open && (
          <motion.div
            initial={{ opacity: 0, y: -4, scale: 0.97 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -4, scale: 0.97 }}
            transition={{ type: "spring", stiffness: 400, damping: 30 }}
            className="absolute right-0 top-full mt-1 z-50 bg-bg-base border border-border rounded-lg shadow-lg p-3 space-y-3 min-w-[180px]"
          >
            <p className="text-[11px] font-medium text-text-tertiary tracking-wide">Export</p>
            <div className="flex gap-1">
              {(["txt", "srt", "json", "csv"] as const).map((fmt) => (
                <button
                  key={fmt}
                  onClick={() => setExportFormat(fmt)}
                  className={`px-2 py-1 text-xs font-mono rounded transition-colors ${
                    exportFormat === fmt
                      ? "bg-accent text-white"
                      : "text-text-tertiary hover:text-text-primary bg-bg-surface"
                  }`}
                >
                  {fmt.toUpperCase()}
                </button>
              ))}
            </div>
            <div className="flex gap-2">
              <button
                onClick={() => { onSave(); setOpen(false) }}
                disabled={exporting}
                className="flex-1 px-3 py-1.5 text-xs font-mono bg-bg-surface border border-border rounded-md text-text-secondary hover:text-text-primary transition-colors disabled:opacity-40"
              >
                Save to file
              </button>
              <button
                onClick={() => { onCopy(); setOpen(false) }}
                disabled={exporting}
                className="flex-1 px-3 py-1.5 text-xs font-mono bg-bg-surface border border-border rounded-md text-text-secondary hover:text-text-primary transition-colors disabled:opacity-40"
              >
                Copy
              </button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  )
}

export function DashboardTab() {
  const [transcripts, setTranscripts] = useState<Transcript[]>([])
  const [search, setSearch] = useState("")
  const [exportFormat, setExportFormat] = useState<"txt" | "srt" | "json" | "csv">("txt")
  const [exporting, setExporting] = useState(false)

  const loadTranscripts = () => {
    if (search.trim()) {
      invoke<Transcript[]>("search_transcripts", { query: search, limit: 50 })
        .then(setTranscripts).catch(() => {})
    } else {
      invoke<Transcript[]>("get_transcripts", { limit: 50 })
        .then(setTranscripts).catch(() => {})
    }
  }

  useEffect(() => {
    loadTranscripts()
    // Refresh when new transcriptions come in
    const unlisten = listen<string>("transcription", () => {
      setTimeout(loadTranscripts, 200)
    })
    return () => { unlisten.then((fn) => fn()) }
  }, [search])

  const handleCopy = (text: string) => {
    navigator.clipboard.writeText(text)
  }

  const handleDelete = (id: number) => {
    invoke("delete_transcript", { id }).then(() => {
      setTranscripts((prev) => prev.filter((t) => t.id !== id))
    }).catch(() => {})
  }

  const handleExportFile = async () => {
    if (transcripts.length === 0) return
    setExporting(true)
    try {
      const content = await invoke<string>("export_transcripts", { format: exportFormat, ids: [] })
      const ext = exportFormat
      const path = await (await import("@tauri-apps/plugin-dialog")).save({
        defaultPath: `inkwell-export.${ext}`,
        filters: [{ name: ext.toUpperCase(), extensions: [ext] }],
      })
      if (path) {
        const { writeTextFile } = await import("@tauri-apps/plugin-fs")
        await writeTextFile(path, content)
      }
    } catch (e) {
      console.error("Export failed:", e)
    } finally {
      setExporting(false)
    }
  }

  const handleExportClipboard = async () => {
    if (transcripts.length === 0) return
    setExporting(true)
    try {
      const content = await invoke<string>("export_transcripts", { format: exportFormat, ids: [] })
      await navigator.clipboard.writeText(content)
    } catch (e) {
      console.error("Clipboard export failed:", e)
    } finally {
      setExporting(false)
    }
  }

  const formatTime = (timestamp: string) => {
    try {
      const d = new Date(timestamp)
      const now = new Date()
      const isToday = d.toDateString() === now.toDateString()
      if (isToday) {
        return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
      }
      return d.toLocaleDateString([], { month: "short", day: "numeric" }) + " " +
        d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    } catch {
      return timestamp
    }
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <h2 className="text-[15px] font-sans font-semibold text-text-primary">Dashboard</h2>
        <span className="text-[11px] font-mono text-text-tertiary">{transcripts.length}</span>

        {/* Export menu */}
        {transcripts.length > 0 && (
          <ExportMenu
            exportFormat={exportFormat}
            setExportFormat={setExportFormat}
            onSave={handleExportFile}
            onCopy={handleExportClipboard}
            exporting={exporting}
          />
        )}
      </div>

      <input
        type="text"
        value={search}
        onChange={(e) => setSearch(e.target.value)}
        placeholder="Search transcripts..."
        aria-label="Search transcripts"
        className="w-full px-3 py-2 text-[13px] bg-bg-surface border border-border rounded-lg text-text-primary placeholder:text-text-tertiary focus:outline-none focus:border-border-default transition-colors"
      />

      {transcripts.length === 0 ? (
        <p className="text-[13px] text-text-tertiary py-12 text-center">
          {search ? "No matching transcripts." : "No transcripts yet. Press your hotkey to start dictating."}
        </p>
      ) : (
        <div className="space-y-1.5">
          <AnimatePresence>
            {transcripts.map((t, i) => (
              <motion.div
                key={t.id}
                initial={{ opacity: 0, y: 6 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, x: -8 }}
                transition={{ type: "spring", stiffness: 350, damping: 30, delay: i * 0.015 }}
                className="group px-3.5 py-3 bg-bg-surface border border-border rounded-lg hover:border-border-default transition-all duration-150"
              >
                <div className="flex items-start justify-between gap-3">
                  <p className="text-[13px] text-text-primary flex-1 leading-[1.6]">{t.text}</p>
                  <div className="flex gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity shrink-0">
                    <button
                      onClick={() => handleCopy(t.text)}
                      aria-label="Copy transcript"
                      className="px-2 py-1 text-[11px] text-text-tertiary hover:text-text-primary rounded transition-colors"
                    >
                      Copy
                    </button>
                    <button
                      onClick={() => handleDelete(t.id)}
                      aria-label="Delete transcript"
                      className="px-2 py-1 text-[11px] text-text-tertiary hover:text-red-400 rounded transition-colors"
                    >
                      Del
                    </button>
                  </div>
                </div>
                <div className="flex items-center gap-2.5 mt-1">
                  <span className="text-[10px] font-mono text-text-tertiary">{formatTime(t.created_at)}</span>
                  <span className="text-[10px] font-mono text-text-tertiary">{(t.audio_duration_ms / 1000).toFixed(1)}s</span>
                  <span className="text-[10px] font-mono text-text-tertiary">{t.model}</span>
                </div>
              </motion.div>
            ))}
          </AnimatePresence>
        </div>
      )}
    </div>
  )
}
