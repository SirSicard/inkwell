import { useState, useEffect } from "react"
import { motion } from "framer-motion"
import { listen } from "@tauri-apps/api/event"
import { invoke } from "@tauri-apps/api/core"

const MODEL_CATALOG = [
  { id: "parakeet", name: "Parakeet V3", company: "NVIDIA", description: "Fast and accurate. 25 European languages, auto-detect.", size: "670 MB", languages: "25 languages" },
  { id: "parakeet-v2", name: "Parakeet V2", company: "NVIDIA", description: "English-specialized. Same architecture as V3 but tuned for English.", size: "670 MB", languages: "English" },
  { id: "whisper-turbo", name: "Whisper Turbo", company: "OpenAI", description: "Balanced accuracy and speed.", size: "800 MB", languages: "99 languages" },
  { id: "whisper-large-v3", name: "Whisper Large V3", company: "OpenAI", description: "Best accuracy, but slow.", size: "1.5 GB", languages: "99 languages" },
  { id: "whisper-medium", name: "Whisper Medium", company: "OpenAI", description: "Good accuracy, medium speed.", size: "1.0 GB", languages: "99 languages" },
  { id: "whisper-small", name: "Whisper Small", company: "OpenAI", description: "Fast and fairly accurate.", size: "375 MB", languages: "99 languages" },
  { id: "whisper-base", name: "Whisper Base", company: "OpenAI", description: "Lightweight multilingual.", size: "135 MB", languages: "99 languages" },
  { id: "whisper-tiny", name: "Whisper Tiny", company: "OpenAI", description: "Smallest Whisper model.", size: "98 MB", languages: "99 languages" },
  { id: "whisper-distil-medium-en", name: "Whisper Distil Medium", company: "OpenAI", description: "English only. Distilled for speed.", size: "460 MB", languages: "English" },
  { id: "whisper-distil-small-en", name: "Whisper Distil Small", company: "OpenAI", description: "English only. Compact and fast.", size: "180 MB", languages: "English" },
  { id: "moonshine-base", name: "Moonshine Base", company: "Useful Sensors", description: "English only. Good accuracy, fast inference. Upgrade from Tiny.", size: "288 MB", languages: "English" },
  { id: "sense-voice", name: "SenseVoice", company: "Alibaba", description: "Very fast. Chinese, English, Japanese, Korean, Cantonese.", size: "160 MB", languages: "5 languages" },
  { id: "moonshine-tiny", name: "Moonshine Tiny", company: "Useful Sensors", description: "English only. Ultrafast bundled fallback.", size: "70 MB", languages: "English" },
] as const

export function ModelsTab() {
  const [activeModel, setActiveModel] = useState("")
  const [installed, setInstalled] = useState<Record<string, boolean>>({})
  const [switching, setSwitching] = useState<string | null>(null)
  const [downloading, setDownloading] = useState<string | null>(null)
  const [downloadPercent, setDownloadPercent] = useState(0)
  const [removing, setRemoving] = useState<string | null>(null)

  const refreshInstalled = () => {
    invoke<Record<string, boolean>>("get_installed_models").then(setInstalled).catch(() => {})
  }

  useEffect(() => {
    invoke<string>("get_model_name").then(setActiveModel).catch(() => {})
    refreshInstalled()
  }, [])

  useEffect(() => {
    const unlisten = listen<{ percent: number; file: string }>("model-download-progress", (e) => {
      setDownloadPercent(e.payload.percent)
      if (e.payload.percent >= 100) {
        setTimeout(() => {
          setDownloading(null)
          setDownloadPercent(0)
          refreshInstalled()
        }, 500)
      }
    })
    return () => { unlisten.then((fn) => fn()) }
  }, [])

  useEffect(() => {
    const unlisten = listen<string>("model-loaded", (e) => {
      setActiveModel(e.payload)
    })
    return () => { unlisten.then((fn) => fn()) }
  }, [])

  const handleRemove = async (modelId: string) => {
    setRemoving(modelId)
    try {
      await invoke("remove_model", { modelId })
      refreshInstalled()
    } catch (e) {
      console.error("Remove failed:", e)
      alert(String(e))
    }
    setRemoving(null)
  }

  const handleDownload = async (modelId: string) => {
    setDownloading(modelId)
    setDownloadPercent(0)
    try {
      await invoke("download_model", { modelId })
      try {
        const name = await invoke<string>("switch_model", { model: modelId })
        setActiveModel(name)
      } catch (_) {}
    } catch (e) {
      console.error("Download failed:", e)
    }
    setDownloading(null)
  }

  const handleSwitch = async (modelId: string) => {
    setSwitching(modelId)
    try {
      const name = await invoke<string>("switch_model", { model: modelId })
      setActiveModel(name)
    } catch (e) {
      console.error("Switch failed:", e)
    }
    setSwitching(null)
  }

  const isActive = (modelId: string) => {
    const catalog = MODEL_CATALOG.find((m) => m.id === modelId)
    if (!catalog) return false
    return activeModel === catalog.name || activeModel.includes(catalog.name)
  }

  // Group by installed/available, then by company
  const installedModels = MODEL_CATALOG.filter((m) => installed[m.id])
  const availableModels = MODEL_CATALOG.filter((m) => !installed[m.id])

  const groupByCompany = (models: typeof MODEL_CATALOG) => {
    const groups: Record<string, typeof MODEL_CATALOG[number][]> = {}
    for (const m of models) {
      if (!groups[m.company]) groups[m.company] = []
      groups[m.company].push(m)
    }
    return Object.entries(groups)
  }

  const ModelCard = ({ model }: { model: typeof MODEL_CATALOG[number] }) => {
    const active = isActive(model.id)
    const isInstalled = installed[model.id] ?? false
    const isSwitching = switching === model.id
    const isDownloading = downloading === model.id
    const isRemoving = removing === model.id

    return (
      <div
        className={`px-4 py-3 rounded-lg border transition-all duration-150 ${
          active
            ? "bg-accent/[0.06] border-accent/25"
            : "bg-bg-surface border-border hover:border-border-default"
        }`}
      >
        <div className="flex items-start justify-between gap-3">
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2">
              <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${active ? "bg-accent" : "bg-text-tertiary"}`} />
              <span className="text-[13px] font-semibold text-text-primary">{model.name}</span>
              <span className="text-[10px] font-mono text-text-tertiary uppercase tracking-wider">{model.company}</span>
            </div>
            <p className="text-[12px] text-text-secondary mt-1 ml-[14px]">{model.description}</p>
            <div className="flex items-center gap-2 mt-1.5 ml-[14px]">
              <span className="text-[10px] text-text-tertiary">{model.languages}</span>
            </div>
          </div>
          <div className="flex items-center gap-2 shrink-0">
            <span className="text-[12px] font-mono text-text-tertiary">{model.size}</span>
            {active ? (
              <span className="text-[11px] font-medium text-accent">Active</span>
            ) : isDownloading ? (
              <div className="w-20 space-y-1">
                <div className="w-full bg-bg-base rounded-full h-1 overflow-hidden">
                  <motion.div className="h-full bg-accent" animate={{ width: `${downloadPercent}%` }} />
                </div>
                <p className="text-[9px] font-mono text-text-tertiary text-right">{downloadPercent}%</p>
              </div>
            ) : isInstalled ? (
              <div className="flex items-center gap-1">
                <button
                  onClick={() => handleSwitch(model.id)}
                  disabled={isSwitching}
                  className="text-[11px] text-text-secondary hover:text-text-primary transition-colors disabled:opacity-50"
                >
                  {isSwitching ? "..." : "Switch"}
                </button>
                <span className="text-border">|</span>
                <button
                  onClick={() => handleRemove(model.id)}
                  disabled={isRemoving}
                  className="text-[11px] text-text-tertiary hover:text-red-400 transition-colors disabled:opacity-50"
                >
                  {isRemoving ? "..." : "Del"}
                </button>
              </div>
            ) : (
              <button
                onClick={() => handleDownload(model.id)}
                className="text-text-tertiary hover:text-text-secondary transition-colors"
                title="Download"
              >
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
                  <path d="M8 2v8m0 0l-3-3m3 3l3-3M3 13h10" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
                </svg>
              </button>
            )}
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className="space-y-6">
      {/* Downloaded models */}
      {installedModels.length > 0 && (
        <div className="space-y-2">
          <p className="text-[11px] font-mono text-text-tertiary uppercase tracking-wider">Downloaded Models</p>
          {installedModels.map((m) => <ModelCard key={m.id} model={m} />)}
        </div>
      )}

      {/* Available to download, grouped by company */}
      {availableModels.length > 0 && (
        <div className="space-y-5">
          <p className="text-[11px] font-mono text-text-tertiary uppercase tracking-wider">Available to Download</p>
          {groupByCompany(availableModels as unknown as typeof MODEL_CATALOG).map(([company, models]) => (
            <div key={company} className="space-y-1.5">
              <p className="text-[10px] font-mono text-text-tertiary uppercase tracking-widest">{company}</p>
              {models.map((m) => <ModelCard key={m.id} model={m} />)}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
