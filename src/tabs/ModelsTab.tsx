import { useState, useEffect } from "react"
import { motion } from "framer-motion"
import { listen } from "@tauri-apps/api/event"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "../state/toasts"

/// Mirrors models::ModelInfo in src-tauri/src/models.rs, which is the single
/// source of truth. This list used to be hardcoded here as well, and the two
/// drifted: moonshine-tiny sat in the UI catalogue with no download arm in the
/// backend, so its Download button just returned "Unknown model".
type ModelInfo = {
  id: string
  name: string
  company: string
  description: string
  size: string
  languages: string
  installed: boolean
}

export function ModelsTab() {
  const [activeModel, setActiveModel] = useState("")
  const [catalog, setCatalog] = useState<ModelInfo[]>([])
  const [switching, setSwitching] = useState<string | null>(null)
  const [downloading, setDownloading] = useState<string | null>(null)
  const [downloadPercent, setDownloadPercent] = useState(0)
  const [removing, setRemoving] = useState<string | null>(null)

  const refreshInstalled = () => {
    invoke<ModelInfo[]>("list_models").then(setCatalog).catch((e) => toast(`Could not load the model list: ${e}`, "warning"))
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
      // Was a raw alert(); the app has a toast system.
      toast(`Could not remove model: ${e}`)
    }
    setRemoving(null)
  }

  const handleDownload = async (modelId: string) => {
    setDownloading(modelId)
    setDownloadPercent(0)
    try {
      await invoke("download_model", { modelId })
      // Switching straight after a download is a convenience, not the point of
      // the action: report it, but a failure here does not mean the download
      // failed.
      try {
        const name = await invoke<string>("switch_model", { model: modelId })
        setActiveModel(name)
      } catch (e) {
        toast(`Downloaded, but could not activate it: ${e}`, "warning")
      }
    } catch (e) {
      toast(`Download failed: ${e}`)
    }
    setDownloading(null)
  }

  const handleSwitch = async (modelId: string) => {
    setSwitching(modelId)
    try {
      const name = await invoke<string>("switch_model", { model: modelId })
      setActiveModel(name)
    } catch (e) {
      toast(`Could not switch model: ${e}`)
    }
    setSwitching(null)
  }

  // The backend sets model_name from the same spec.display this list carries,
  // so an exact match is enough; no substring guessing.
  const isActive = (modelId: string) =>
    catalog.find((m) => m.id === modelId)?.name === activeModel

  // Group by installed/available, then by company
  const installedModels = catalog.filter((m) => m.installed)
  const availableModels = catalog.filter((m) => !m.installed)

  const groupByCompany = (models: ModelInfo[]) => {
    const groups: Record<string, ModelInfo[]> = {}
    for (const m of models) {
      if (!groups[m.company]) groups[m.company] = []
      groups[m.company].push(m)
    }
    return Object.entries(groups)
  }

  const ModelCard = ({ model }: { model: ModelInfo }) => {
    const active = isActive(model.id)
    const isInstalled = model.installed
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
          {groupByCompany(availableModels).map(([company, models]) => (
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
