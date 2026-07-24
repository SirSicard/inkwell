import { useState, useEffect } from "react"
import { motion } from "framer-motion"
import { listen } from "@tauri-apps/api/event"
import { invoke } from "@tauri-apps/api/core"
import type { FileTranscribeResult } from "../types"

function formatSrtTime(ms: number): string {
  const h = Math.floor(ms / 3600000)
  const m = Math.floor((ms % 3600000) / 60000)
  const s = Math.floor((ms % 60000) / 1000)
  const millis = ms % 1000
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")},${String(millis).padStart(3, "0")}`
}

export function FilesTab() {
  const [dragging, setDragging] = useState(false)
  const [processing, setProcessing] = useState(false)
  const [progress, setProgress] = useState<{ phase: string; percent: number; filename?: string; chunk?: number; total_chunks?: number } | null>(null)
  const [results, setResults] = useState<FileTranscribeResult[]>([])
  const [error, setError] = useState("")

  // Listen for drag-drop events from Tauri
  useEffect(() => {
    const listeners = [
      listen<{ paths: string[] }>("tauri://drag-drop", async (e) => {
        setDragging(false)
        const paths = e.payload.paths || []
        if (paths.length === 0) return

        setError("")
        setProcessing(true)
        setProgress({ phase: "starting", percent: 0 })

        for (const path of paths) {
          try {
            const result = await invoke<FileTranscribeResult>("transcribe_file", { path })
            setResults((prev) => [result, ...prev])
          } catch (err) {
            setError(String(err))
          }
        }

        setProcessing(false)
        setProgress(null)
      }),
      listen("tauri://drag-enter", () => setDragging(true)),
      listen("tauri://drag-leave", () => setDragging(false)),
      listen<{ phase: string; percent: number; filename?: string; chunk?: number; total_chunks?: number }>(
        "file-transcribe-progress",
        (e) => setProgress(e.payload)
      ),
    ]
    return () => { listeners.forEach((p) => p.then((fn) => fn())) }
  }, [])

  // Also allow file picker
  const handlePickFile = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog")
      const selected = await open({
        multiple: true,
        filters: [{ name: "Audio/Video", extensions: ["mp3", "wav", "flac", "ogg", "m4a", "aac", "mp4", "mov", "mkv", "webm"] }],
      })
      if (!selected) return
      const paths = Array.isArray(selected) ? selected : [selected]

      setError("")
      setProcessing(true)
      setProgress({ phase: "starting", percent: 0 })

      for (const path of paths) {
        try {
          const result = await invoke<FileTranscribeResult>("transcribe_file", { path })
          setResults((prev) => [result, ...prev])
        } catch (err) {
          setError(String(err))
        }
      }

      setProcessing(false)
      setProgress(null)
    } catch (err) {
      setError(String(err))
    }
  }

  const handleCopy = (text: string) => {
    navigator.clipboard.writeText(text)
  }

  const handleExportSrt = async (result: FileTranscribeResult) => {
    const lines = result.segments.map((seg, i) => {
      const start = formatSrtTime(seg.start_ms)
      const end = formatSrtTime(seg.end_ms)
      return `${i + 1}\n${start} --> ${end}\n${seg.text.trim()}\n`
    }).join("\n")

    try {
      const { save } = await import("@tauri-apps/plugin-dialog")
      const { writeTextFile } = await import("@tauri-apps/plugin-fs")
      const name = result.filename.replace(/\.[^.]+$/, ".srt")
      const path = await save({ defaultPath: name, filters: [{ name: "SRT", extensions: ["srt"] }] })
      if (path) await writeTextFile(path, lines)
    } catch (e) {
      console.error("SRT export failed:", e)
    }
  }

  const phaseLabel = (phase: string) => {
    switch (phase) {
      case "decoding": return "Decoding audio..."
      case "analyzing": return "Detecting speech..."
      case "transcribing": return progress?.chunk
        ? `Transcribing (${progress.chunk}/${progress.total_chunks})...`
        : "Transcribing..."
      case "complete": return "Done!"
      default: return "Starting..."
    }
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <h2 className="text-[15px] font-sans font-semibold text-text-primary">File Transcription</h2>
        {!processing && (
          <button
            onClick={handlePickFile}
            className="text-xs font-mono text-text-tertiary hover:text-text-secondary transition-colors"
          >
            Browse files
          </button>
        )}
      </div>

      {/* Drop zone */}
      <motion.div
        animate={dragging ? { scale: 1.02, borderColor: "rgba(255,255,255,0.3)" } : { scale: 1 }}
        className={`relative border-2 border-dashed rounded-xl p-8 text-center transition-colors ${
          dragging
            ? "border-accent bg-accent/5"
            : processing
            ? "border-border bg-bg-surface/80"
            : "border-border hover:border-border-default"
        }`}
      >
        {processing && progress ? (
          <div className="space-y-3">
            <p className="text-sm text-text-secondary">{phaseLabel(progress.phase)}</p>
            {progress.filename && (
              <p className="text-xs font-mono text-text-tertiary">{progress.filename}</p>
            )}
            <div className="w-full max-w-xs mx-auto bg-bg-base border border-border rounded-full h-1.5 overflow-hidden">
              <motion.div
                className="h-full bg-accent"
                initial={{ width: 0 }}
                animate={{ width: `${progress.percent}%` }}
                transition={{ ease: "linear" }}
              />
            </div>
            <p className="text-[10px] font-mono text-text-tertiary">{progress.percent}%</p>
          </div>
        ) : (
          <div className="space-y-2">
            <p className="text-sm text-text-secondary">
              {dragging ? "Drop to transcribe" : "Drag audio or video files here"}
            </p>
            <p className="text-xs text-text-tertiary">
              MP3, WAV, FLAC, OGG, M4A, MP4, MKV, WebM
            </p>
          </div>
        )}
      </motion.div>

      {error && (
        <div className="p-3 bg-red-950/80 border border-red-800/50 rounded-lg">
          <p className="text-xs text-red-200">{error}</p>
        </div>
      )}

      {/* Results */}
      {results.length > 0 && (
        <div className="space-y-3">
          <p className="text-[11px] font-medium text-text-tertiary tracking-wide">Results</p>
          {results.map((r, i) => (
            <motion.div
              key={`${r.filename}-${i}`}
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ type: "spring", stiffness: 300, damping: 30 }}
              className="group p-4 bg-bg-surface border border-border rounded-lg space-y-2"
            >
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <span className="text-sm font-medium text-text-primary">{r.filename}</span>
                  <span className="text-xs font-mono text-text-tertiary">{r.duration_s.toFixed(1)}s</span>
                  <span className="text-xs font-mono text-text-tertiary">{r.segments.length} segments</span>
                </div>
                <div className="flex gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                  <button
                    onClick={() => handleCopy(r.text)}
                    className="px-2 py-1 text-xs text-text-tertiary hover:text-text-primary transition-colors"
                  >
                    Copy
                  </button>
                  <button
                    onClick={() => handleExportSrt(r)}
                    className="px-2 py-1 text-xs text-text-tertiary hover:text-text-primary transition-colors"
                  >
                    SRT
                  </button>
                </div>
              </div>
              <p className="text-sm text-text-secondary leading-relaxed whitespace-pre-wrap">{r.text}</p>
            </motion.div>
          ))}
        </div>
      )}
    </div>
  )
}
