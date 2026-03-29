import { useState, useEffect, useRef } from "react"
import { motion, AnimatePresence } from "framer-motion"
import { listen } from "@tauri-apps/api/event"
import { invoke } from "@tauri-apps/api/core"
import { InkCanvas } from "./components/InkCanvas"
import { SettingRow, GlassToggle, GlassSelect, GlassButton } from "./components/ui"

const basicTabs = ["Dashboard", "General", "About"] as const
const advancedTabs = ["Dashboard", "General", "Audio", "Models", "AI", "Snippets", "App Styles", "Dictionary", "Files", "Commands", "About"] as const
type Tab = (typeof advancedTabs)[number]

function Onboarding({ onComplete }: { onComplete: () => void }) {
  const [step, setStep] = useState(0)
  const [hotkeyTested, setHotkeyTested] = useState(false)
  const [firstTranscription, setFirstTranscription] = useState("")
  const [downloadState, setDownloadState] = useState<"idle" | "downloading" | "done" | "skipped">("idle")
  const [downloadPercent, setDownloadPercent] = useState(0)
  const [downloadFile, setDownloadFile] = useState("")

  useEffect(() => {
    const unlisten = listen<string>("transcription", (e) => {
      if (e.payload && !firstTranscription) {
        setFirstTranscription(e.payload)
        setHotkeyTested(true)
      }
    })
    return () => { unlisten.then((fn) => fn()) }
  }, [firstTranscription])

  useEffect(() => {
    const unlisten = listen<{ percent: number; file: string }>("model-download-progress", (e) => {
      setDownloadPercent(e.payload.percent)
      setDownloadFile(e.payload.file)
      if (e.payload.percent >= 100) {
        setDownloadState("done")
      }
    })
    return () => { unlisten.then((fn) => fn()) }
  }, [])

  const startDownload = () => {
    setDownloadState("downloading")
    setDownloadPercent(0)
    invoke("download_parakeet").catch((e) => {
      console.error("Download failed:", e)
      setDownloadState("idle")
    })
  }

  const steps = [
    // Step 0: Welcome
    <div key="welcome" className="text-center space-y-5">
      <div className="text-6xl font-sans font-bold tracking-tight">INKWELL</div>
      <p className="text-text-secondary text-base">Premium speech-to-text. Private by default.</p>
      <p className="text-text-tertiary text-sm">Everything runs locally on your machine. No data leaves your computer.</p>
    </div>,

    // Step 1: Mic check
    <div key="mic" className="text-center space-y-5">
      <div className="text-3xl font-sans font-semibold">Microphone</div>
      <p className="text-text-secondary text-base leading-relaxed">
        Inkwell needs access to your microphone to transcribe speech.
        Your audio is processed locally and never sent anywhere.
      </p>
      <p className="text-text-tertiary text-sm">
        If your browser asks for mic permission, click Allow.
      </p>
    </div>,

    // Step 2: Better model offer
    <div key="model" className="text-center space-y-5">
      <div className="text-3xl font-sans font-semibold">Better Accuracy</div>
      <p className="text-text-secondary text-base leading-relaxed">
        Inkwell ships with a lightweight model that works right away.
        For significantly better accuracy, download the Parakeet V3 model.
      </p>
      <div className="bg-bg-surface border border-border rounded-lg p-4 text-left space-y-3">
        <div className="flex justify-between text-sm font-mono">
          <span className="text-text-secondary">Moonshine Tiny</span>
          <span className="text-text-tertiary">31 MB · running now</span>
        </div>
        <div className="flex justify-between text-sm font-mono">
          <span className="text-text-primary font-medium">Parakeet V3</span>
          <span className="text-accent">670 MB · recommended</span>
        </div>
      </div>

      {downloadState === "idle" && (
        <div className="flex justify-center gap-3">
          <button
            onClick={startDownload}
            className="px-6 py-2.5 text-sm font-medium bg-accent text-white rounded-lg hover:bg-accent/90 transition-colors"
          >
            Download Parakeet V3
          </button>
          <button
            onClick={() => setDownloadState("skipped")}
            className="px-5 py-2.5 text-sm text-text-tertiary hover:text-text-secondary transition-colors"
          >
            Skip for now
          </button>
        </div>
      )}

      {downloadState === "downloading" && (
        <div className="space-y-2">
          <div className="w-full bg-bg-surface border border-border rounded-full h-1.5 overflow-hidden">
            <motion.div
              className="h-full bg-accent"
              initial={{ width: 0 }}
              animate={{ width: `${downloadPercent}%` }}
              transition={{ ease: "linear" }}
            />
          </div>
          <p className="text-sm font-mono text-text-tertiary">
            {downloadPercent}% · {downloadFile !== "done" ? downloadFile : "finishing up..."}
          </p>
        </div>
      )}

      {(downloadState === "done" || downloadState === "skipped") && (
        <motion.p
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          className="text-base text-text-secondary"
        >
          {downloadState === "done"
            ? "Parakeet V3 downloaded. It will load on next restart."
            : "No problem. You can download models anytime in the Models tab."}
        </motion.p>
      )}
    </div>,

    // Step 3: Hotkey test
    <div key="hotkey" className="text-center space-y-5">
      <div className="text-3xl font-sans font-semibold">Try It</div>
      <p className="text-text-secondary text-base leading-relaxed">
        Hold <span className="font-mono bg-bg-surface px-2.5 py-1 rounded-md border border-border text-sm">Ctrl + Space</span> and say something.
      </p>
      {hotkeyTested ? (
        <motion.div
          initial={{ opacity: 0, scale: 0.9 }}
          animate={{ opacity: 1, scale: 1 }}
          className="space-y-3"
        >
          <p className="text-text-primary text-base font-medium">It worked!</p>
          <p className="text-text-secondary text-sm italic">"{firstTranscription}"</p>
        </motion.div>
      ) : (
        <p className="text-text-tertiary text-sm animate-pulse">Waiting for your voice...</p>
      )}
    </div>,

    // Step 4: Done
    <div key="done" className="text-center space-y-5">
      <div className="text-3xl font-sans font-semibold">You're all set</div>
      <p className="text-text-secondary text-base leading-relaxed">
        Inkwell lives in your system tray. Close the window and it keeps working.
        Press your hotkey from any app to transcribe.
      </p>
      <div className="bg-bg-surface border border-border rounded-lg p-4 text-left space-y-3">
        <p className="text-sm font-medium text-text-primary">Want more power?</p>
        <p className="text-sm text-text-secondary leading-relaxed">
          Enable <span className="font-medium text-text-primary">Advanced Mode</span> in
          General settings to unlock AI Polish, voice commands, file transcription, 
          snippets, per-app styles, and more.
        </p>
      </div>
    </div>,
  ]

  const isLastStep = step === steps.length - 1
  // Step 2 (model): can proceed once user picked download or skip
  // Step 3 (try it): must get a successful transcription
  const canProceed =
    step === 2 ? (downloadState === "done" || downloadState === "skipped") :
    step === 3 ? hotkeyTested :
    true

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="fixed inset-0 z-50 bg-bg-primary/95 backdrop-blur-md flex items-center justify-center"
    >
      <motion.div
        key={step}
        initial={{ opacity: 0, y: 20 }}
        animate={{ opacity: 1, y: 0 }}
        exit={{ opacity: 0, y: -20 }}
        transition={{ type: "spring", stiffness: 300, damping: 30 }}
        className="max-w-lg w-full px-10"
      >
        {steps[step]}

        <div className="flex justify-center gap-3 mt-10">
          {step > 0 && (
            <button
              onClick={() => setStep(step - 1)}
              className="px-5 py-2.5 text-sm text-text-tertiary hover:text-text-secondary transition-colors"
            >
              Back
            </button>
          )}
          <button
            onClick={() => isLastStep ? onComplete() : setStep(step + 1)}
            disabled={!canProceed && !isLastStep}
            className="px-7 py-2.5 text-sm font-medium bg-bg-surface border border-border rounded-lg text-text-primary hover:border-border-default transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
          >
            {isLastStep ? "Start Using Inkwell" : step === 0 ? "Get Started" : "Next"}
          </button>
        </div>

        {/* Step dots */}
        <div className="flex justify-center gap-1.5 mt-6">
          {steps.map((_, i) => (
            <div
              key={i}
              className={`w-1.5 h-1.5 rounded-full transition-colors ${
                i === step ? "bg-text-primary" : "bg-border"
              }`}
            />
          ))}
        </div>
      </motion.div>
    </motion.div>
  )
}

interface Toast {
  id: number
  message: string
  type: "error" | "warning" | "info"
}

let toastId = 0

function TabBar({ tabs, activeTab, onTabChange }: { tabs: readonly string[]; activeTab: string; onTabChange: (t: Tab) => void }) {
  const containerRef = useRef<HTMLDivElement>(null)
  const [showMenu, setShowMenu] = useState(false)
  const [containerWidth, setContainerWidth] = useState(999)

  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    const observer = new ResizeObserver((entries) => {
      setContainerWidth(entries[0].contentRect.width)
    })
    observer.observe(el)
    return () => observer.disconnect()
  }, [])

  // ~80px per tab on average. Reserve 40px for hamburger.
  const maxVisible = Math.max(1, Math.floor((containerWidth - 40) / 80))
  const needsOverflow = tabs.length > maxVisible
  const visibleTabs = needsOverflow ? tabs.slice(0, maxVisible) : tabs
  const overflowTabs = needsOverflow ? tabs.slice(maxVisible) : []

  return (
    <div ref={containerRef} role="tablist" className="relative flex items-center gap-0.5 px-5 pt-3 pb-0 border-b border-border"
      onKeyDown={(e) => {
        const allTabs = [...tabs]
        const idx = allTabs.indexOf(activeTab)
        if (e.key === "ArrowRight" && idx < allTabs.length - 1) { e.preventDefault(); onTabChange(allTabs[idx + 1] as Tab) }
        if (e.key === "ArrowLeft" && idx > 0) { e.preventDefault(); onTabChange(allTabs[idx - 1] as Tab) }
        if (e.key === "Home") { e.preventDefault(); onTabChange(allTabs[0] as Tab) }
        if (e.key === "End") { e.preventDefault(); onTabChange(allTabs[allTabs.length - 1] as Tab) }
      }}
    >
      {visibleTabs.map((tab) => (
        <button
          key={tab}
          role="tab"
          aria-selected={activeTab === tab}
          tabIndex={activeTab === tab ? 0 : -1}
          onClick={() => { onTabChange(tab as Tab); setShowMenu(false) }}
          className={`relative px-3 py-2.5 text-[11px] font-medium tracking-wide transition-colors whitespace-nowrap focus:outline-none ${
            activeTab === tab
              ? "text-text-primary"
              : "text-text-tertiary hover:text-text-secondary"
          }`}
        >
          {tab}
          {activeTab === tab && (
            <motion.div
              layoutId="tab-underline"
              className="absolute bottom-0 left-3 right-3 h-[2px] bg-accent rounded-full"
              transition={{ type: "spring", stiffness: 400, damping: 35 }}
            />
          )}
        </button>
      ))}

      {needsOverflow && (
        <div className="ml-auto relative pb-2">
          <button
            onClick={() => setShowMenu(!showMenu)}
            className={`flex items-center justify-center w-7 h-7 rounded-md transition-colors ${
              showMenu || overflowTabs.includes(activeTab)
                ? "text-text-primary bg-bg-hover"
                : "text-text-tertiary hover:text-text-secondary"
            }`}
          >
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none">
              <circle cx="3" cy="8" r="1.2" fill="currentColor" />
              <circle cx="8" cy="8" r="1.2" fill="currentColor" />
              <circle cx="13" cy="8" r="1.2" fill="currentColor" />
            </svg>
          </button>
          <AnimatePresence>
            {showMenu && (
              <motion.div
                initial={{ opacity: 0, y: -4, scale: 0.97 }}
                animate={{ opacity: 1, y: 0, scale: 1 }}
                exit={{ opacity: 0, y: -4, scale: 0.97 }}
                transition={{ type: "spring", stiffness: 400, damping: 30 }}
                className="absolute right-0 top-full mt-1 z-50 min-w-[140px] bg-bg-overlay border border-border-default rounded-lg shadow-xl py-1"
              >
                {overflowTabs.map((tab) => (
                  <button
                    key={tab}
                    onClick={() => { onTabChange(tab as Tab); setShowMenu(false) }}
                    className={`w-full text-left px-3 py-2 text-[11px] font-medium tracking-wide transition-colors ${
                      activeTab === tab
                        ? "text-text-primary bg-bg-hover"
                        : "text-text-tertiary hover:text-text-secondary hover:bg-bg-hover"
                    }`}
                  >
                    {tab}
                  </button>
                ))}
              </motion.div>
            )}
          </AnimatePresence>
        </div>
      )}
    </div>
  )
}

// --- Update state ---
interface UpdateInfo {
  version: string
  notes: string
  date: string
}

function App() {
  const [activeTab, setActiveTab] = useState<Tab>("Dashboard")
  const [modelName, setModelName] = useState("Loading...")
  const [advancedMode, setAdvancedMode] = useState(false)
  const [toasts, setToasts] = useState<Toast[]>([])
  const [updateAvailable, setUpdateAvailable] = useState<UpdateInfo | null>(null)
  const [updateDismissed, setUpdateDismissed] = useState(false)
  const [updateProgress, setUpdateProgress] = useState<number | null>(null)

  const addToast = (message: string, type: Toast["type"] = "error") => {
    const id = ++toastId
    setToasts((prev) => [...prev, { id, message, type }])
    setTimeout(() => setToasts((prev) => prev.filter((t) => t.id !== id)), 5000)
  }

  // Check for updates on mount (real Tauri updater)
  useEffect(() => {
    let cancelled = false
    const checkUpdate = async () => {
      try {
        const { check } = await import("@tauri-apps/plugin-updater")
        const update = await check()
        if (update && !cancelled) {
          setUpdateAvailable({
            version: update.version,
            notes: update.body || "Bug fixes and improvements.",
            date: update.date?.split("T")[0] || "",
          })
          // Store the update object for later download
          ;(window as any).__inkwell_update = update
        }
      } catch (e) {
        console.log("Update check skipped:", e)
      }
    }
    // Delay check by 5s to not slow startup
    const timer = setTimeout(checkUpdate, 5000)
    return () => { cancelled = true; clearTimeout(timer) }
  }, [])

  // First-run onboarding
  const [showOnboarding, setShowOnboarding] = useState(false)
  useEffect(() => {
    invoke<boolean>("check_first_run").then((first) => {
      if (first) setShowOnboarding(true)
    }).catch(() => {})
  }, [])

  // Listen for error events from Rust backend
  useEffect(() => {
    const listeners = [
      listen<string>("transcription-error", (e) => addToast(`Transcription failed: ${e.payload}`)),
      listen<string>("paste-error", (e) => addToast(`Paste failed. Text copied to clipboard: ${e.payload}`, "warning")),
      listen<string>("mic-error", (e) => addToast(`Mic error: ${e.payload}`, "warning")),
      listen<string>("model-error", (e) => addToast(`Model error: ${e.payload}`, "warning")),
    ]
    return () => { listeners.forEach((p) => p.then((fn) => fn())) }
  }, [])

  useEffect(() => {
    invoke<Settings>("get_settings").then((s) => {
      setAdvancedMode(s.advanced_mode)
    }).catch(() => {})
  }, [])

  const tabs = advancedMode ? advancedTabs : basicTabs

  // Reset tab if it's not available in current mode
  useEffect(() => {
    if (!(tabs as readonly string[]).includes(activeTab)) {
      setActiveTab("Dashboard")
    }
  }, [advancedMode])

  useEffect(() => {
    // Query model name on mount (event fires before webview is ready)
    invoke<string>("get_model_name").then(setModelName).catch(() => {})
    // Also listen for future model changes
    const unlisten = listen<string>("model-loaded", (event) => {
      setModelName(event.payload)
    })
    return () => { unlisten.then((fn) => fn()) }
  }, [])

  return (
    <div className="h-screen flex bg-bg-base">
      {/* Left: Ink Zone */}
      <div className="w-[35%] min-w-[240px] h-full relative bg-ink-bg flex flex-col overflow-visible">
        {/* Ink canvas */}
        <div className="flex-1 relative overflow-hidden">
          <InkCanvas />

          {/* Overlay text with blend mode difference */}
          <div className="absolute inset-0 pointer-events-none flex justify-center pt-8">
            <span className="mix-blend-difference text-5xl font-sans font-bold uppercase tracking-tight text-white">
              INKWELL
            </span>
          </div>
        </div>

        {/* Update toast - positioned on the outer ink panel */}
        <AnimatePresence>
          {updateAvailable && !updateDismissed && (
            <motion.div
              initial={{ opacity: 0, y: 30 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: 20, scale: 0.95 }}
              transition={{ type: "spring", stiffness: 260, damping: 24, delay: 0.3 }}
              className="absolute bottom-4 left-1/2 -translate-x-1/2 z-30 w-[85%] max-w-[280px]"
            >
              <div className="bg-bg-surface/95 backdrop-blur-xl border border-border rounded-xl shadow-2xl overflow-hidden">
                <div className="h-0.5 bg-gradient-to-r from-emerald-500 via-emerald-400 to-teal-500" />
                <div className="p-4 space-y-3">
                  <div className="flex items-start justify-between">
                    <div className="flex items-center gap-2.5">
                      <div className="w-8 h-8 rounded-lg bg-emerald-500/10 border border-emerald-500/20 flex items-center justify-center shrink-0">
                        <svg width="16" height="16" viewBox="0 0 16 16" fill="none" className="text-emerald-400">
                          <path d="M8 1v10M4 7l4 4 4-4" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"/>
                          <path d="M2 13h12" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"/>
                        </svg>
                      </div>
                      <div>
                        <p className="text-sm font-medium text-text-primary">Update available</p>
                        <p className="text-[11px] font-mono text-emerald-400">v{updateAvailable.version}</p>
                      </div>
                    </div>
                    <button
                      onClick={() => setUpdateDismissed(true)}
                      className="text-text-tertiary hover:text-text-secondary transition-colors p-0.5 -m-0.5"
                    >
                      <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
                        <path d="M3 3l8 8M11 3l-8 8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"/>
                      </svg>
                    </button>
                  </div>
                  <p className="text-xs text-text-secondary leading-relaxed">{updateAvailable.notes}</p>
                  {updateProgress !== null && (
                    <div className="space-y-1">
                      <div className="h-1.5 rounded-full bg-bg-base overflow-hidden">
                        <motion.div
                          className="h-full rounded-full bg-gradient-to-r from-emerald-500 to-teal-500"
                          initial={{ width: 0 }}
                          animate={{ width: `${updateProgress}%` }}
                          transition={{ duration: 0.3 }}
                        />
                      </div>
                      <p className="text-[10px] font-mono text-text-tertiary text-right">{updateProgress}%</p>
                    </div>
                  )}
                  {updateProgress === null && (
                    <div className="flex items-center gap-2 pt-0.5">
                      <button
                        onClick={async () => {
                          const update = (window as any).__inkwell_update
                          if (!update) return
                          setUpdateProgress(0)
                          try {
                            let downloaded = 0
                            const total = update.rawContentLength || 1
                            await update.downloadAndInstall((event: any) => {
                              if (event.event === "Started" && event.data?.contentLength) {
                                // total known
                              } else if (event.event === "Progress") {
                                downloaded += event.data?.chunkLength || 0
                                setUpdateProgress(Math.min(Math.round((downloaded / total) * 100), 99))
                              } else if (event.event === "Finished") {
                                setUpdateProgress(100)
                              }
                            })
                            setUpdateProgress(null)
                            setUpdateDismissed(true)
                            addToast("Update installed. Restart to apply.", "info")
                            // Prompt restart
                            const { relaunch } = await import("@tauri-apps/plugin-process")
                            await relaunch()
                          } catch (e) {
                            console.error("Update failed:", e)
                            setUpdateProgress(null)
                            addToast(`Update failed: ${e}`, "error")
                          }
                        }}
                        className="flex-1 px-3 py-1.5 text-xs font-medium text-white bg-emerald-600 hover:bg-emerald-500 rounded-lg transition-colors"
                      >
                        Update now
                      </button>
                      <button
                        onClick={() => setUpdateDismissed(true)}
                        className="px-3 py-1.5 text-xs text-text-tertiary hover:text-text-secondary transition-colors"
                      >
                        Later
                      </button>
                    </div>
                  )}
                </div>
              </div>
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      {/* Right: Content Zone */}
      <div className="flex-1 h-full flex flex-col border-l border-border bg-bg-base">
        {/* Tab Bar */}
        <TabBar tabs={tabs} activeTab={activeTab} onTabChange={setActiveTab} />

        {/* Content Area */}
        <div role="tabpanel" className="flex-1 overflow-y-auto px-5 py-5">
          <AnimatePresence mode="wait">
            <motion.div
              key={activeTab}
              initial={{ opacity: 0, y: 4 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.12 }}
            >
              <TabContent tab={activeTab} onAdvancedChange={setAdvancedMode} />
            </motion.div>
          </AnimatePresence>
        </div>

        {/* Status Bar */}
        <div className="flex items-center justify-between px-5 py-2.5 border-t border-border">
          <span className="text-[11px] font-mono text-text-tertiary tracking-wide flex items-center gap-1.5">
            {modelName === "Loading..." || modelName === "No model loaded" ? (
              <><span className="inline-block w-1.5 h-1.5 rounded-full bg-amber-400 animate-pulse" />{modelName}</>
            ) : (
              <><span className="inline-block w-1.5 h-1.5 rounded-full bg-green-400" />{modelName}</>
            )}
          </span>
          <span className="text-[11px] font-mono text-text-tertiary">v0.1.0</span>
        </div>
      </div>

      {/* Onboarding */}
      <AnimatePresence>
        {showOnboarding && (
          <Onboarding onComplete={() => setShowOnboarding(false)} />
        )}
      </AnimatePresence>

      {/* Toasts */}
      <div className="fixed bottom-4 right-4 z-50 flex flex-col gap-2 max-w-sm">
        <AnimatePresence>
          {toasts.map((toast) => (
            <motion.div
              key={toast.id}
              initial={{ opacity: 0, y: 20, scale: 0.95 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, y: -10, scale: 0.95 }}
              transition={{ type: "spring", stiffness: 300, damping: 30 }}
              className={`px-4 py-3 rounded-lg border text-sm backdrop-blur-sm ${
                toast.type === "error"
                  ? "bg-red-950/80 border-red-800/50 text-red-200"
                  : toast.type === "warning"
                  ? "bg-amber-950/80 border-amber-800/50 text-amber-200"
                  : "bg-bg-surface border-border text-text-primary"
              }`}
              onClick={() => setToasts((prev) => prev.filter((t) => t.id !== toast.id))}
            >
              {toast.message}
            </motion.div>
          ))}
        </AnimatePresence>
      </div>


    </div>
  )
}

function TabContent({ tab, onAdvancedChange }: { tab: Tab; onAdvancedChange?: (v: boolean) => void }) {
  switch (tab) {
    case "Dashboard":
      return <DashboardTab />
    case "General":
      return <GeneralTab onAdvancedChange={onAdvancedChange} />
    case "Audio":
      return <AudioTab />
    case "Models":
      return <ModelsTab />
    case "Dictionary":
      return <DictionaryTab />
    case "AI":
      return <AITab />
    case "Snippets":
      return <SnippetsTab />
    case "App Styles":
      return <AppStylesTab />
    case "Files":
      return <FilesTab />
    case "Commands":
      return <VoiceCommandsTab />
    case "About":
      return <AboutTab />
  }
}

interface Transcript {
  id: number
  text: string
  raw_text: string
  style: string
  model: string
  audio_duration_ms: number
  created_at: string
}

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

function DashboardTab() {
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

function ModelsTab() {
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
interface DeviceInfo {
  id: string
  name: string
}

function AudioTab() {
  const [devices, setDevices] = useState<DeviceInfo[]>([])
  const [selectedMic, setSelectedMic] = useState("auto")
  const [vadThreshold, setVadThreshold] = useState(0.5)

  useEffect(() => {
    invoke<DeviceInfo[]>("get_input_devices").then(setDevices).catch(() => {})
    invoke<Settings>("get_settings").then((s) => {
      setSelectedMic(s.mic_device)
      setVadThreshold(s.vad_threshold)
    }).catch(() => {})
  }, [])

  const handleMicChange = (value: string) => {
    setSelectedMic(value)
    invoke("update_settings", { key: "mic_device", value }).catch(() => {})
  }

  const handleVadChange = (value: number) => {
    setVadThreshold(value)
    invoke("set_vad_threshold", { threshold: value }).catch(() => {})
  }

  const vadLabel = vadThreshold < 0.3 ? "Very sensitive" :
    vadThreshold < 0.5 ? "Sensitive" :
    vadThreshold < 0.7 ? "Balanced" :
    vadThreshold < 0.85 ? "Strict" : "Very strict"

  return (
    <div className="space-y-3">
      <h2 className="text-[15px] font-sans font-semibold text-text-primary">Audio</h2>

      <SettingRow label="Microphone" description="Which input device to use">
        <GlassSelect
          value={selectedMic}
          onChange={handleMicChange}
          options={[
            { label: "Auto (skip virtual)", value: "auto" },
            ...devices.map((d) => ({ label: d.name, value: d.id })),
          ]}
        />
      </SettingRow>

      <div className="p-4 bg-bg-surface border border-border rounded-lg space-y-3">
        <div className="flex items-center justify-between">
          <div>
            <div className="text-sm text-text-primary">VAD Sensitivity</div>
            <div className="text-xs text-text-tertiary">{vadLabel} ({vadThreshold.toFixed(2)})</div>
          </div>
        </div>
        <div className="flex items-center gap-3">
          <span className="text-xs text-text-tertiary shrink-0">Sensitive</span>
          <input
            type="range"
            min="0.1"
            max="0.95"
            step="0.05"
            value={vadThreshold}
            aria-label="VAD sensitivity"
            onChange={(e) => handleVadChange(parseFloat(e.target.value))}
            className="flex-1 h-1.5 bg-border rounded-full appearance-none cursor-pointer
              [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:w-3.5 [&::-webkit-slider-thumb]:h-3.5
              [&::-webkit-slider-thumb]:bg-text-secondary [&::-webkit-slider-thumb]:rounded-full
              [&::-webkit-slider-thumb]:hover:bg-text-primary [&::-webkit-slider-thumb]:transition-colors"
          />
          <span className="text-xs text-text-tertiary shrink-0">Strict</span>
        </div>
      </div>

      <div className="p-3 bg-bg-surface border border-border rounded-lg">
        <h3 className="text-[11px] font-medium text-text-tertiary tracking-wide mb-2">Detected devices</h3>
        {devices.length === 0 ? (
          <p className="text-sm text-text-tertiary">No input devices found. Check your mic connection.</p>
        ) : (
          <div className="space-y-1.5">
            {devices.map((d, i) => (
              <div key={i} className="flex items-center gap-2">
                <span className="text-xs font-mono text-text-tertiary">[{i}]</span>
                <span className="text-sm text-text-secondary">{d.name}</span>
              </div>
            ))}
          </div>
        )}
      </div>

      <p className="text-xs text-text-tertiary">
        Mic changes take effect on next restart. VAD sensitivity applies immediately.
      </p>
    </div>
  )
}

interface DictEntry {
  find: string
  replace: string
}

interface AppStyleRule {
  process_name: string
  style: string
}

interface VoiceCommandItem {
  id: string
  triggers: string[]
  action: { type: string; style?: string; model?: string; url?: string; path?: string; text?: string }
  enabled: boolean
}

interface VoiceCommandStoreData {
  enabled: boolean
  wake_prefix: string
  commands: VoiceCommandItem[]
}

function VoiceCommandsTab() {
  const [store, setStore] = useState<VoiceCommandStoreData | null>(null)
  const [lastCommand, setLastCommand] = useState("")

  useEffect(() => {
    invoke<VoiceCommandStoreData>("get_voice_commands").then(setStore).catch(() => {})
    const unlisten = listen<{ id: string }>("voice-command", (e) => {
      setLastCommand(e.payload.id)
      setTimeout(() => setLastCommand(""), 3000)
    })
    return () => { unlisten.then((fn) => fn()) }
  }, [])

  const saveStore = (updated: VoiceCommandStoreData) => {
    setStore(updated)
    invoke("save_voice_commands", { store: updated }).catch(() => {})
  }

  const toggleEnabled = (enabled: boolean) => {
    if (!store) return
    saveStore({ ...store, enabled })
  }

  const toggleCommand = (id: string) => {
    if (!store) return
    saveStore({
      ...store,
      commands: store.commands.map((c) =>
        c.id === id ? { ...c, enabled: !c.enabled } : c
      ),
    })
  }

  const updateWakePrefix = (wake_prefix: string) => {
    if (!store) return
    saveStore({ ...store, wake_prefix })
  }

  const removeCommand = (id: string) => {
    if (!store) return
    saveStore({ ...store, commands: store.commands.filter((c) => c.id !== id) })
  }

  // Add custom command
  const [showAdd, setShowAdd] = useState(false)
  const [newTrigger, setNewTrigger] = useState("")
  const [newActionType, setNewActionType] = useState("open_url")
  const [newActionValue, setNewActionValue] = useState("")

  const handleAdd = () => {
    if (!store || !newTrigger.trim()) return
    const triggers = newTrigger.split(",").map((t) => t.trim().toLowerCase()).filter(Boolean)
    if (triggers.length === 0) return

    let action: VoiceCommandItem["action"]
    switch (newActionType) {
      case "open_url":
        action = { type: "open_url", url: newActionValue }
        break
      case "open_app":
        action = { type: "open_app", path: newActionValue }
        break
      case "insert_text":
        action = { type: "insert_text", text: newActionValue }
        break
      case "change_style":
        action = { type: "change_style", style: newActionValue || "formal" }
        break
      default:
        return
    }

    const cmd: VoiceCommandItem = {
      id: `custom-${Date.now()}`,
      triggers,
      action,
      enabled: true,
    }

    saveStore({ ...store, commands: [...store.commands, cmd] })
    setNewTrigger("")
    setNewActionValue("")
    setShowAdd(false)
  }

  const actionLabel = (action: VoiceCommandItem["action"]) => {
    switch (action.type) {
      case "undo": return "Undo last transcription"
      case "change_style": return `Style → ${action.style}`
      case "switch_model": return `Model → ${action.model}`
      case "toggle_polish": return "Toggle AI Polish"
      case "toggle_dictation": return "Pause/resume dictation"
      case "open_url": return `Open ${action.url}`
      case "open_app": return `Launch ${action.path}`
      case "insert_text": return `Insert "${action.text?.slice(0, 30)}..."`
      default: return action.type
    }
  }

  if (!store) return <p className="text-sm text-text-tertiary">Loading...</p>

  return (
    <div className="space-y-4">
      {/* Master toggle */}
      <div className="p-4 bg-bg-surface border border-border rounded-xl space-y-3">
        <div className="flex items-center justify-between">
          <div>
            <p className="text-sm font-medium text-text-primary">Voice Commands</p>
            <p className="text-xs text-text-tertiary mt-0.5">
              {store.enabled
                ? `Say "${store.wake_prefix}" followed by a command`
                : "Disabled. All speech is treated as dictation."
              }
            </p>
          </div>
          <GlassToggle checked={store.enabled} onChange={toggleEnabled} />
        </div>

        {store.enabled && (
          <div className="flex items-center gap-2">
            <span className="text-xs text-text-tertiary shrink-0">Wake word:</span>
            <input
              value={store.wake_prefix}
              onChange={(e) => updateWakePrefix(e.target.value.toLowerCase())}
              className="w-32 px-2 py-1 text-xs font-mono bg-bg-base border border-border rounded-md text-text-primary focus:outline-none focus:border-border-default"
            />
          </div>
        )}
      </div>

      {/* Last triggered indicator */}
      <AnimatePresence>
        {lastCommand && (
          <motion.div
            initial={{ opacity: 0, y: -8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0 }}
            className="px-3 py-2 bg-accent/10 border border-accent/20 rounded-lg"
          >
            <p className="text-xs text-accent">Command triggered: {lastCommand}</p>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Command list */}
      <div className="space-y-1.5">
        {store.commands.map((cmd) => (
          <div
            key={cmd.id}
            className={`group flex items-center gap-3 p-3 bg-bg-surface border border-border rounded-lg transition-colors ${
              !cmd.enabled ? "opacity-40" : ""
            } ${lastCommand === cmd.id ? "border-accent/30 bg-accent/5" : ""}`}
          >
            <button
              onClick={() => toggleCommand(cmd.id)}
              className={`w-2 h-2 rounded-full shrink-0 transition-colors ${
                cmd.enabled ? "bg-green-400" : "bg-border"
              }`}
            />
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2 flex-wrap">
                {cmd.triggers.map((t, i) => (
                  <span key={i} className="text-xs font-mono text-accent">
                    "{t}"
                    {i < cmd.triggers.length - 1 && <span className="text-text-tertiary ml-1">/</span>}
                  </span>
                ))}
              </div>
              <p className="text-[10px] text-text-tertiary mt-0.5">{actionLabel(cmd.action)}</p>
            </div>
            {cmd.id.startsWith("custom-") && (
              <button
                onClick={() => removeCommand(cmd.id)}
                className="px-2 py-1 text-xs text-text-tertiary hover:text-red-400 transition-colors opacity-0 group-hover:opacity-100"
              >
                Del
              </button>
            )}
          </div>
        ))}
      </div>

      {/* Add custom command */}
      {!showAdd ? (
        <button
          onClick={() => setShowAdd(true)}
          className="w-full py-2 text-xs text-text-tertiary hover:text-text-secondary border border-dashed border-border rounded-lg transition-colors"
        >
          + Add custom command
        </button>
      ) : (
        <motion.div
          initial={{ opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          className="p-3 bg-bg-surface border border-border rounded-lg space-y-2"
        >
          <input
            value={newTrigger}
            onChange={(e) => setNewTrigger(e.target.value)}
            placeholder="Trigger phrases (comma-separated)"
            className="w-full px-3 py-1.5 text-xs bg-bg-base border border-border rounded-md text-text-primary placeholder:text-text-tertiary focus:outline-none focus:border-border-default font-mono"
          />
          <div className="flex gap-2">
            <select
              value={newActionType}
              onChange={(e) => setNewActionType(e.target.value)}
              className="px-2 py-1.5 text-xs bg-bg-base border border-border rounded-md text-text-primary focus:outline-none font-mono"
            >
              <option value="open_url">Open URL</option>
              <option value="open_app">Open App</option>
              <option value="insert_text">Insert Text</option>
              <option value="change_style">Change Style</option>
            </select>
            <input
              value={newActionValue}
              onChange={(e) => setNewActionValue(e.target.value)}
              placeholder={
                newActionType === "open_url" ? "https://..." :
                newActionType === "open_app" ? "notepad.exe" :
                newActionType === "insert_text" ? "Text to insert" :
                "formal / casual / relaxed"
              }
              className="flex-1 px-3 py-1.5 text-xs bg-bg-base border border-border rounded-md text-text-primary placeholder:text-text-tertiary focus:outline-none focus:border-border-default font-mono"
            />
          </div>
          <div className="flex gap-2">
            <button
              onClick={handleAdd}
              disabled={!newTrigger.trim()}
              className="px-3 py-1.5 text-xs font-mono bg-accent text-white rounded-md hover:bg-accent/90 transition-colors disabled:opacity-40"
            >
              Add
            </button>
            <button
              onClick={() => setShowAdd(false)}
              className="px-3 py-1.5 text-xs text-text-tertiary hover:text-text-secondary transition-colors"
            >
              Cancel
            </button>
          </div>
        </motion.div>
      )}

      <p className="text-xs text-text-tertiary">
        Say "{store.wake_prefix}, [command]" during any recording. Commands are detected before dictation is processed.
      </p>
    </div>
  )
}

interface FileTranscribeResult {
  filename: string
  duration_s: number
  text: string
  raw_text: string
  segments: { start_ms: number; end_ms: number; text: string }[]
}

function FilesTab() {
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

function formatSrtTime(ms: number): string {
  const h = Math.floor(ms / 3600000)
  const m = Math.floor((ms % 3600000) / 60000)
  const s = Math.floor((ms % 60000) / 1000)
  const millis = ms % 1000
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")},${String(millis).padStart(3, "0")}`
}

function AppStylesTab() {
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

interface SnippetItem {
  id: string
  trigger: string
  expansion: string
  category: string
  enabled: boolean
}

function SnippetsTab() {
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

function AIConsentModal({ onAccept, onDecline }: { onAccept: () => void; onDecline: () => void }) {
  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-6"
    >
      <motion.div
        initial={{ opacity: 0, y: 24, scale: 0.97 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        exit={{ opacity: 0, y: 12 }}
        transition={{ type: "spring", stiffness: 300, damping: 28 }}
        className="max-w-md w-full bg-bg-base border border-border rounded-xl p-6 space-y-5"
      >
        <div className="space-y-1">
          <h3 className="text-base font-sans font-medium text-text-primary">Before you enable AI Polish</h3>
          <p className="text-xs text-text-tertiary">A quick heads-up about how this works.</p>
        </div>

        <div className="space-y-3 text-sm text-text-secondary">
          <p>AI Polish sends your transcription text to an LLM to clean up grammar, punctuation, and capitalization.</p>
          <p className="text-text-tertiary text-xs">Your audio never leaves your device. Only the final text is sent.</p>
        </div>

        <div className="bg-bg-surface border border-border rounded-lg p-4 space-y-2">
          <div className="flex items-baseline gap-2">
            <span className="text-2xl font-sans font-bold text-text-primary">4,000</span>
            <span className="text-sm text-text-secondary">words/week free</span>
          </div>
          <p className="text-xs text-text-tertiary">Resets every Monday. No account needed. Add your own API key for unlimited.</p>
        </div>

        <div className="flex gap-3 pt-1">
          <button onClick={onAccept} className="flex-1 px-4 py-2.5 text-sm bg-accent text-white rounded-lg hover:bg-accent/90 transition-colors font-medium">
            Enable AI Polish
          </button>
          <button onClick={onDecline} className="px-4 py-2.5 text-sm text-text-tertiary hover:text-text-secondary transition-colors">
            Not now
          </button>
        </div>
      </motion.div>
    </motion.div>
  )
}

function AITab() {
  const [provider, setProvider] = useState("groq")
  const [apiKey, setApiKey] = useState("")
  const [keyStatus, setKeyStatus] = useState<Record<string, boolean>>({})
  const [polishEnabled, setPolishEnabled] = useState(false)
  const [showConsent, setShowConsent] = useState(false)
  const [polishPrompt, setPolishPrompt] = useState("")
  const [usage, setUsage] = useState({ words_used: 0, free_tier: 4000, remaining: 4000, over_limit: false, week_start: "" })
  const [saving, setSaving] = useState(false)
  const [testText, setTestText] = useState("")
  const [testResult, setTestResult] = useState("")
  const [testing, setTesting] = useState(false)
  const [showPrompt, setShowPrompt] = useState(false)
  const [showTest, setShowTest] = useState(false)

  const providers = [
    { id: "groq", label: "Groq", hint: "llama-3.3-70b", free: true },
    { id: "openai", label: "OpenAI", hint: "gpt-4o-mini" },
    { id: "anthropic", label: "Anthropic", hint: "Claude Haiku" },
    { id: "openrouter", label: "OpenRouter", hint: "any model" },
  ] as const

  const hasAnyKey = Object.values(keyStatus).some(Boolean)

  const handleTogglePolish = (enabled: boolean) => {
    if (enabled && !polishEnabled) {
      setShowConsent(true)
    } else {
      setPolishEnabled(false)
      invoke("set_polish_settings", { enabled: false, prompt: polishPrompt }).catch(() => {})
    }
  }

  const handleConsentAccept = () => {
    setShowConsent(false)
    setPolishEnabled(true)
    invoke("set_polish_settings", { enabled: true, prompt: polishPrompt }).catch(() => {})
  }

  useEffect(() => {
    invoke<Record<string, boolean>>("get_api_key_status").then(setKeyStatus).catch(() => {})
    invoke<{ enabled: boolean; prompt: string }>("get_polish_settings").then((s) => {
      setPolishEnabled(s.enabled)
      setPolishPrompt(s.prompt)
    }).catch(() => {})
    invoke<typeof usage>("get_usage").then(setUsage).catch(() => {})
  }, [])

  const handleSaveKey = async () => {
    setSaving(true)
    try {
      await invoke("save_api_key", { provider, key: apiKey })
      const status = await invoke<Record<string, boolean>>("get_api_key_status")
      setKeyStatus(status)
      setApiKey("")
    } catch (e) { console.error(e) }
    setSaving(false)
  }

  const handleSavePolish = () => {
    invoke("set_polish_settings", { enabled: polishEnabled, prompt: polishPrompt }).catch(() => {})
  }

  const handleTest = async () => {
    if (!testText.trim()) return
    setTesting(true)
    setTestResult("")
    try {
      const useProvider = keyStatus[provider] ? provider : null
      const result = await invoke<{ text: string; words_used?: number; remaining?: number }>(
        "run_ai_polish", { text: testText, provider: useProvider, model: null }
      )
      setTestResult(result.text)
      if (result.remaining !== undefined) {
        setUsage((u) => ({ ...u, remaining: result.remaining!, words_used: result.words_used ?? u.words_used }))
      }
    } catch (e) {
      setTestResult(`Error: ${e}`)
    }
    setTesting(false)
  }

  const usagePct = Math.min((usage.words_used / usage.free_tier) * 100, 100)

  return (
    <div className="space-y-4">
      {/* Consent modal */}
      <AnimatePresence>
        {showConsent && (
          <AIConsentModal
            onAccept={handleConsentAccept}
            onDecline={() => setShowConsent(false)}
          />
        )}
      </AnimatePresence>

      {/* -- Section 1: Master toggle -- */}
      <div className="p-4 bg-bg-surface border border-border rounded-xl space-y-3">
        <div className="flex items-center justify-between">
          <div>
            <p className="text-sm font-medium text-text-primary">AI Polish</p>
            <p className="text-xs text-text-tertiary mt-0.5">
              {polishEnabled
                ? hasAnyKey
                  ? "Using your API key"
                  : `Free tier: ${usage.remaining.toLocaleString()} words left this week`
                : "Off. Transcriptions are raw."
              }
            </p>
          </div>
          <GlassToggle checked={polishEnabled} onChange={handleTogglePolish} />
        </div>

        {/* Inline usage bar (only when enabled + free tier) */}
        {polishEnabled && !hasAnyKey && (
          <div className="space-y-1">
            <div className="w-full bg-bg-base rounded-full h-1 overflow-hidden">
              <motion.div
                className={`h-full ${usage.over_limit ? "bg-red-400" : "bg-accent"}`}
                initial={{ width: 0 }}
                animate={{ width: `${usagePct}%` }}
                transition={{ duration: 0.5 }}
              />
            </div>
            <div className="flex justify-between">
              <span className="text-[10px] font-mono text-text-tertiary">
                {usage.words_used.toLocaleString()} / {usage.free_tier.toLocaleString()}
              </span>
              {usage.over_limit && (
                <span className="text-[10px] text-red-400">Limit reached. Add an API key below.</span>
              )}
            </div>
          </div>
        )}
      </div>

      {/* -- Section 2: API Keys -- */}
      <div className="p-4 bg-bg-surface border border-border rounded-xl space-y-3">
        <div>
          <p className="text-sm font-medium text-text-primary">API Keys</p>
          <p className="text-xs text-text-tertiary mt-0.5">
            {hasAnyKey ? "Your key, direct to the model. Unlimited." : "Add a key for unlimited use. Never touches our servers."}
          </p>
        </div>

        {/* Provider row */}
        <div className="flex gap-1.5 flex-wrap">
          {providers.map((p) => (
            <button
              key={p.id}
              onClick={() => setProvider(p.id)}
              className={`flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border transition-all ${
                provider === p.id
                  ? "border-text-tertiary/40 text-text-primary bg-bg-base"
                  : "border-transparent text-text-tertiary hover:text-text-secondary"
              }`}
            >
              {p.label}
              {"free" in p && p.free && !keyStatus[p.id] && (
                <span className="text-[10px] px-1 py-0.5 rounded bg-green-400/10 text-green-400">free</span>
              )}
              {keyStatus[p.id] && (
                <span className="w-1.5 h-1.5 rounded-full bg-green-400" title="Configured" />
              )}
            </button>
          ))}
        </div>

        {/* Key input */}
        <div className="flex gap-2">
          <input
            type="password"
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && apiKey.trim() && handleSaveKey()}
            placeholder={keyStatus[provider] ? `${providers.find(p => p.id === provider)?.label} key saved` : `Paste ${providers.find(p => p.id === provider)?.label} API key...`}
            className={`flex-1 px-3 py-2 text-sm bg-bg-base border rounded-lg text-text-primary placeholder:text-text-tertiary focus:outline-none transition-colors ${
              keyStatus[provider] ? "border-green-800/30" : "border-border focus:border-border-default"
            }`}
          />
          {apiKey.trim() ? (
            <button
              onClick={handleSaveKey}
              disabled={saving}
              className="px-4 py-2 text-xs font-mono bg-accent text-white rounded-lg hover:bg-accent/90 transition-colors disabled:opacity-40"
            >
              {saving ? "..." : "Save"}
            </button>
          ) : keyStatus[provider] ? (
            <button
              onClick={() => { invoke("save_api_key", { provider, key: "" }).then(() => invoke<Record<string, boolean>>("get_api_key_status").then(setKeyStatus)) }}
              className="px-3 py-2 text-xs font-mono text-red-400/70 hover:text-red-400 transition-colors"
            >
              Remove
            </button>
          ) : null}
        </div>

        <p className="text-[10px] text-text-tertiary">
          {providers.find(p => p.id === provider)?.hint}
          {provider === "groq" && !keyStatus.groq && " \u00b7 get a free key at console.groq.com"}
        </p>
      </div>

      {/* -- Section 3: Collapsible prompt editor -- */}
      <div className="border border-border rounded-xl overflow-hidden">
        <button
          onClick={() => setShowPrompt(!showPrompt)}
          className="w-full flex items-center justify-between px-4 py-3 text-left hover:bg-bg-surface/80 transition-colors"
        >
          <div>
            <p className="text-sm text-text-secondary">Polish Prompt</p>
            <p className="text-[10px] text-text-tertiary">How the LLM processes your text</p>
          </div>
          <motion.span
            animate={{ rotate: showPrompt ? 180 : 0 }}
            className="text-text-tertiary text-xs"
          >
            &#9662;
          </motion.span>
        </button>
        <AnimatePresence>
          {showPrompt && (
            <motion.div
              initial={{ height: 0, opacity: 0 }}
              animate={{ height: "auto", opacity: 1 }}
              exit={{ height: 0, opacity: 0 }}
              transition={{ duration: 0.2 }}
              className="overflow-hidden"
            >
              <div className="px-4 pb-4 space-y-2">
                <textarea
                  value={polishPrompt}
                  onChange={(e) => setPolishPrompt(e.target.value)}
                  onBlur={handleSavePolish}
                  rows={3}
                  className="w-full px-3 py-2 text-xs bg-bg-base border border-border rounded-lg text-text-secondary focus:outline-none focus:border-border-default resize-none font-mono leading-relaxed"
                />
              </div>
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      {/* -- Section 4: Collapsible test panel -- */}
      <div className="border border-border rounded-xl overflow-hidden">
        <button
          onClick={() => setShowTest(!showTest)}
          className="w-full flex items-center justify-between px-4 py-3 text-left hover:bg-bg-surface/80 transition-colors"
        >
          <div>
            <p className="text-sm text-text-secondary">Test</p>
            <p className="text-[10px] text-text-tertiary">Try AI Polish on sample text</p>
          </div>
          <motion.span
            animate={{ rotate: showTest ? 180 : 0 }}
            className="text-text-tertiary text-xs"
          >
            &#9662;
          </motion.span>
        </button>
        <AnimatePresence>
          {showTest && (
            <motion.div
              initial={{ height: 0, opacity: 0 }}
              animate={{ height: "auto", opacity: 1 }}
              exit={{ height: 0, opacity: 0 }}
              transition={{ duration: 0.2 }}
              className="overflow-hidden"
            >
              <div className="px-4 pb-4 space-y-2">
                <textarea
                  value={testText}
                  onChange={(e) => setTestText(e.target.value)}
                  placeholder="Paste raw transcription text..."
                  rows={2}
                  className="w-full px-3 py-2 text-xs bg-bg-base border border-border rounded-lg text-text-secondary placeholder:text-text-tertiary focus:outline-none focus:border-border-default resize-none"
                />
                <div className="flex items-center gap-2">
                  <button
                    onClick={handleTest}
                    disabled={testing || !testText.trim()}
                    className="px-4 py-1.5 text-xs font-mono bg-bg-base border border-border text-text-secondary hover:text-text-primary rounded-lg transition-colors disabled:opacity-40"
                  >
                    {testing ? "Polishing..." : "Run"}
                  </button>
                  {testResult && !testing && (
                    <span className="text-[10px] text-text-tertiary">
                      {hasAnyKey ? `via ${provider}` : "via free tier"}
                    </span>
                  )}
                </div>
                {testResult && (
                  <motion.div
                    initial={{ opacity: 0, y: 4 }}
                    animate={{ opacity: 1, y: 0 }}
                    className="p-3 bg-bg-base border border-border rounded-lg"
                  >
                    <p className="text-sm text-text-primary leading-relaxed">{testResult}</p>
                  </motion.div>
                )}
              </div>
            </motion.div>
          )}
        </AnimatePresence>
      </div>
    </div>
  )
}


function DictionaryTab() {
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

interface Settings {
  style: string
  model: string
  hotkey: string
  recording_mode: string
  start_on_boot: boolean
  show_overlay: boolean
  advanced_mode: boolean
  mic_device: string
  vad_threshold: number
}

function HotkeyCapture() {
  const [hotkey, setHotkey] = useState("ctrl+space")
  const [capturing, setCapturing] = useState(false)
  const [error, setError] = useState("")

  useEffect(() => {
    invoke<Settings>("get_settings").then((s) => setHotkey(s.hotkey)).catch(() => {})
  }, [])

  const formatKey = (e: KeyboardEvent) => {
    const parts: string[] = []
    if (e.ctrlKey) parts.push("ctrl")
    if (e.altKey) parts.push("alt")
    if (e.shiftKey) parts.push("shift")
    if (e.metaKey) parts.push("super")

    const key = e.key.toLowerCase()
    // Skip standalone modifier keys
    if (["control", "alt", "shift", "meta"].includes(key)) return null
    // Map special keys
    const keyMap: Record<string, string> = {
      " ": "space", "arrowup": "up", "arrowdown": "down",
      "arrowleft": "left", "arrowright": "right", "escape": "escape",
      "enter": "enter", "backspace": "backspace", "delete": "delete",
      "tab": "tab",
    }
    parts.push(keyMap[key] || key)
    return parts.join("+")
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (!capturing) return
    e.preventDefault()
    e.stopPropagation()

    const combo = formatKey(e.nativeEvent)
    if (!combo) return // just a modifier, wait for the actual key

    setCapturing(false)
    setError("")

    // Must have at least one modifier
    if (!e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) {
      setError("Needs at least one modifier (Ctrl, Alt, Shift)")
      return
    }

    invoke("set_hotkey", { hotkey: combo })
      .then(() => {
        setHotkey(combo)
        setError("")
      })
      .catch((err) => {
        setError(String(err))
      })
  }

  const displayHotkey = capturing ? "Press your hotkey..." : hotkey.split("+").map(
    (k) => k.charAt(0).toUpperCase() + k.slice(1)
  ).join(" + ")

  return (
    <div className="space-y-1">
      <label className="text-xs text-text-tertiary uppercase tracking-wider">Hotkey</label>
      <div
        tabIndex={0}
        onClick={() => { setCapturing(true); setError("") }}
        onKeyDown={handleKeyDown}
        onBlur={() => setCapturing(false)}
        className={`px-3 py-2 text-sm rounded-lg border cursor-pointer transition-colors focus:outline-none ${
          capturing
            ? "bg-bg-surface border-text-tertiary text-text-primary animate-pulse"
            : "bg-bg-surface border-border text-text-primary hover:border-border-default"
        }`}
      >
        {displayHotkey}
      </div>
      {error && <p className="text-xs text-red-400">{error}</p>}
    </div>
  )
}

function GeneralTab({ onAdvancedChange }: { onAdvancedChange?: (v: boolean) => void }) {
  const [startOnBoot, setStartOnBoot] = useState(false)
  const [showOverlay, setShowOverlay] = useState(true)
  const [recordingMode, setRecordingMode] = useState("ptt")
  const [style, setStyle] = useState("formal")
  const [advancedMode, setAdvancedMode] = useState(false)

  useEffect(() => {
    invoke<Settings>("get_settings").then((s) => {
      setStyle(s.style)
      setRecordingMode(s.recording_mode)
      setStartOnBoot(s.start_on_boot)
      setShowOverlay(s.show_overlay)
      setAdvancedMode(s.advanced_mode)
    }).catch(() => {})
  }, [])

  const updateSetting = (key: string, value: string) => {
    invoke("update_settings", { key, value }).catch((e) => console.error("update_settings failed:", e))
  }

  const handleStyleChange = (value: string) => {
    setStyle(value)
    invoke("set_style", { styleName: value }).catch((e) => console.error("set_style failed:", e))
  }

  const handleRecordingModeChange = (value: string) => {
    setRecordingMode(value)
    updateSetting("recording_mode", value)
  }

  const handleStartOnBootChange = (value: boolean) => {
    setStartOnBoot(value)
    updateSetting("start_on_boot", value.toString())
  }

  const handleShowOverlayChange = (value: boolean) => {
    setShowOverlay(value)
    updateSetting("show_overlay", value.toString())
  }

  const handleAdvancedModeChange = (value: boolean) => {
    setAdvancedMode(value)
    updateSetting("advanced_mode", value.toString())
    onAdvancedChange?.(value)
  }

  return (
    <div className="space-y-3">
      <h2 className="text-[15px] font-sans font-semibold text-text-primary">General</h2>

      <SettingRow label="Recording Mode" description="How the hotkey behaves">
        <GlassSelect
          value={recordingMode}
          onChange={handleRecordingModeChange}
          options={[
            { label: "Toggle", value: "toggle" },
            { label: "Push to Talk", value: "ptt" },
          ]}
        />
      </SettingRow>

      {/* Style cards */}
      <div className="space-y-2">
        <div>
          <p className="text-[13px] font-medium text-text-primary">Text Style</p>
          <p className="text-[11px] text-text-tertiary mt-0.5">Controls how your transcribed text is formatted</p>
        </div>
        <div className="grid grid-cols-3 gap-2">
          {([
            { id: "formal", label: "Formal.", sub: "Caps + Punctuation", example: "Hey, are you free for lunch tomorrow? Let's do 12 if that works for you." },
            { id: "casual", label: "Casual", sub: "Caps + Less punctuation", example: "Hey are you free for lunch tomorrow? Lets do 12 if that works for you" },
            { id: "relaxed", label: "very casual", sub: "No caps + Minimal punctuation", example: "hey are you free for lunch tomorrow, lets do 12 if that works for you" },
          ] as const).map((s) => (
            <button
              key={s.id}
              onClick={() => handleStyleChange(s.id)}
              className={`text-left p-3 rounded-lg border transition-all duration-150 ${
                style === s.id
                  ? "bg-accent/[0.06] border-accent/25"
                  : "bg-bg-surface border-border hover:border-border-default"
              }`}
            >
              <p className={`text-[13px] font-semibold ${style === s.id ? "text-text-primary" : "text-text-secondary"}`}>{s.label}</p>
              <p className="text-[10px] text-text-tertiary mt-0.5">{s.sub}</p>
              <div className={`mt-2.5 p-2.5 rounded-md text-[11px] leading-relaxed ${
                style === s.id
                  ? "bg-bg-base text-text-secondary"
                  : "bg-bg-hover/50 text-text-tertiary"
              }`}>
                {s.example}
              </div>
            </button>
          ))}
        </div>
      </div>

      <SettingRow label="Start on Boot" description="Launch Inkwell when you log in">
        <GlassToggle checked={startOnBoot} onChange={handleStartOnBootChange} />
      </SettingRow>

      <SettingRow label="Show Overlay" description="Floating indicator while recording">
        <GlassToggle checked={showOverlay} onChange={handleShowOverlayChange} />
      </SettingRow>

      <SettingRow label="Advanced Mode" description="Show all tabs and settings">
        <GlassToggle checked={advancedMode} onChange={handleAdvancedModeChange} />
      </SettingRow>

      <HotkeyCapture />

      <div className="flex gap-2 pt-2">
        <GlassButton variant="ghost" onClick={() => {
          invoke("update_settings", { key: "style", value: "formal" }).catch(() => {})
          invoke("update_settings", { key: "recording_mode", value: "ptt" }).catch(() => {})
          invoke("update_settings", { key: "show_overlay", value: "true" }).catch(() => {})
          invoke("update_settings", { key: "start_on_boot", value: "false" }).catch(() => {})
          window.location.reload()
        }}>Reset Defaults</GlassButton>
      </div>
    </div>
  )
}

function AboutTab() {
  return (
    <div className="flex items-center justify-center h-full min-h-[400px]">
      <div className="text-center space-y-4">
        <div className="text-4xl font-sans font-bold tracking-tight text-text-primary">INKWELL</div>
        <p className="text-base text-text-secondary">Your voice. Your machine. Nothing leaves.</p>
        <div className="pt-3 space-y-1.5">
          <p className="text-sm text-text-tertiary">Built by Mattias H.</p>
          <p className="text-[11px] font-mono text-text-tertiary">v0.1.0</p>
        </div>
        <div className="pt-4 flex items-center justify-center gap-4">
          <span className="text-xs text-text-tertiary">Tauri + React + sherpa-onnx</span>
        </div>
      </div>
    </div>
  )
}

export default App
