import { useState, useEffect, useRef } from "react"
import { motion, AnimatePresence } from "framer-motion"
import { listen } from "@tauri-apps/api/event"
import { invoke } from "@tauri-apps/api/core"
import { InkCanvas } from "./components/InkCanvas"
import type { Settings, Toast, UpdateInfo, Tab } from "./types"
import { basicTabs, advancedTabs } from "./types"
import {
  DashboardTab, GeneralTab, AudioTab, ModelsTab,
  AITab, AgentTab, SnippetsTab, AppStylesTab, DictionaryTab,
  FilesTab, VoiceCommandsTab, AboutTab,
} from "./tabs"

// --- Onboarding ---

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

// --- Tab Bar ---

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

// --- Tab Router ---

function TabContent({ tab, onAdvancedChange }: { tab: Tab; onAdvancedChange?: (v: boolean) => void }) {
  switch (tab) {
    case "Dashboard":   return <DashboardTab />
    case "General":     return <GeneralTab onAdvancedChange={onAdvancedChange} />
    case "Audio":       return <AudioTab />
    case "Models":      return <ModelsTab />
    case "AI":          return <AITab />
    case "Agent":       return <AgentTab />
    case "Snippets":    return <SnippetsTab />
    case "App Styles":  return <AppStylesTab />
    case "Dictionary":  return <DictionaryTab />
    case "Files":       return <FilesTab />
    case "Commands":    return <VoiceCommandsTab />
    case "About":       return <AboutTab />
  }
}

// --- App ---

let toastId = 0

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

  // Check for updates on mount
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
          ;(window as any).__inkwell_update = update
        }
      } catch (e) {
        console.log("Update check skipped:", e)
      }
    }
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

  // Error event listeners
  useEffect(() => {
    const listeners = [
      listen<string>("transcription-error", (e) => addToast(`Transcription failed: ${e.payload}`)),
      listen<string>("paste-error", (e) => addToast(`Paste failed. Text copied to clipboard: ${e.payload}`, "warning")),
      listen<string>("mic-error", (e) => addToast(`Mic error: ${e.payload}`, "warning")),
      listen<string>("model-error", (e) => addToast(`Model error: ${e.payload}`, "warning")),
    ]
    return () => { listeners.forEach((p) => p.then((fn) => fn())) }
  }, [])

  // Load advanced mode setting
  useEffect(() => {
    invoke<Settings>("get_settings").then((s) => {
      setAdvancedMode(s.advanced_mode)
    }).catch(() => {})
  }, [])

  const tabs = advancedMode ? advancedTabs : basicTabs

  // Reset tab if not available in current mode
  useEffect(() => {
    if (!(tabs as readonly string[]).includes(activeTab)) {
      setActiveTab("Dashboard")
    }
  }, [advancedMode])

  // Model name tracking
  useEffect(() => {
    invoke<string>("get_model_name").then(setModelName).catch(() => {})
    const unlisten = listen<string>("model-loaded", (event) => {
      setModelName(event.payload)
    })
    return () => { unlisten.then((fn) => fn()) }
  }, [])

  return (
    <div className="h-screen flex bg-bg-base">
      {/* Left: Ink Zone */}
      <div className="w-[35%] min-w-[240px] h-full relative bg-ink-bg flex flex-col overflow-visible">
        <div className="flex-1 relative overflow-hidden">
          <InkCanvas />
          <div className="absolute inset-0 pointer-events-none flex justify-center pt-8">
            <span className="mix-blend-difference text-5xl font-sans font-bold uppercase tracking-tight text-white">
              INKWELL
            </span>
          </div>
        </div>

        {/* Update toast */}
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
        <TabBar tabs={tabs} activeTab={activeTab} onTabChange={setActiveTab} />

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

export default App
