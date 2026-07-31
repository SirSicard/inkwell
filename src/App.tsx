import { useState, useEffect, useRef } from "react"
import { motion, AnimatePresence } from "framer-motion"
import { listen } from "@tauri-apps/api/event"
import { invoke } from "@tauri-apps/api/core"
import { getVersion } from "@tauri-apps/api/app"
import type { Update } from "@tauri-apps/plugin-updater"
import { InkCanvas } from "./components/InkCanvas"
import type { Settings, UpdateInfo, Tab } from "./types"
import { formatHotkey } from "./hotkey"
import { useToasts, toast } from "./state/toasts"
import { useSettings } from "./state/settings"
import { watchTheme, type ThemeChoice } from "./theme"
import { basicTabs, advancedTabs, tabGroups } from "./types"
import {
  DashboardTab, GeneralTab, AudioTab, ModelsTab,
  AITab, SnippetsTab, ModesTab, DictionaryTab,
  FilesTab, VoiceCommandsTab, TroubleshootingTab, AboutTab,
} from "./tabs"

// The platform cannot change while the app is running, so this is computed once
// rather than per render.
const IS_MAC = typeof navigator !== "undefined" && navigator.userAgent.includes("Mac")

// --- Onboarding ---

function Onboarding({ onComplete }: { onComplete: () => void }) {
  const [step, setStep] = useState(0)
  // Read the real hotkey rather than hardcoding it: the default differs per platform.
  const [hotkeyLabel, setHotkeyLabel] = useState("")
  const [hotkeyTested, setHotkeyTested] = useState(false)
  const [firstTranscription, setFirstTranscription] = useState("")
  const [downloadState, setDownloadState] = useState<"idle" | "downloading" | "done" | "skipped">("idle")
  const [downloadPercent, setDownloadPercent] = useState(0)
  const [downloadFile, setDownloadFile] = useState("")
  const [micDevices, setMicDevices] = useState<{ id: string; name: string }[]>([])
  const [selectedMic, setSelectedMic] = useState("auto")
  // "unknown" also covers a backend that doesn't expose the command yet. In that
  // case we simply don't claim anything about the permission.
  const [accessibility, setAccessibility] = useState<"unknown" | "granted" | "denied">("unknown")
  const [accessChecking, setAccessChecking] = useState(false)

  // Accessibility is a macOS-only concept; the Windows build has no such gate.
  // Module scope, not component scope: the value cannot change at runtime, and
  // hoisting it keeps it out of every effect's dependency list honestly rather
  // than by suppressing the rule.

  const checkAccessibility = () => {
    setAccessChecking(true)
    invoke<boolean>("check_accessibility_permission")
      .then((ok) => setAccessibility(ok ? "granted" : "denied"))
      .catch(() => setAccessibility("unknown"))
      .finally(() => setAccessChecking(false))
  }

  useEffect(() => {
    invoke<{ id: string; name: string }[]>("get_input_devices").then(setMicDevices).catch(() => {})
    invoke<Settings>("get_settings")
      .then((s) => setHotkeyLabel(formatHotkey(s.hotkey)))
      .catch(() => {})
    // Deferred a tick: calling it inline made this a synchronous setState
    // inside an effect body, which cascades renders.
    if (IS_MAC) queueMicrotask(checkAccessibility)
  }, [])

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
    invoke("download_model", { modelId: "parakeet" }).catch((e) => {
      console.error("Download failed:", e)
      setDownloadState("idle")
    })
  }

  const steps = [
    // Step 0: Welcome
    <div key="welcome" className="text-center space-y-5">
      <div className="text-6xl font-sans font-bold tracking-tight">INKWELL</div>
      {/* "Premium speech-to-text" survived from the version that planned a paid
          tier. It is the first line a new user reads, and it contradicted the
          whole repositioning: free, open source, no tier to upgrade to. */}
      <p className="text-text-secondary text-base">Free speech-to-text. Private by default.</p>
      <p className="text-text-tertiary text-sm">Everything runs locally on your machine. No data leaves your computer.</p>
    </div>,

    // Step 1: Mic check + device selector
    <div key="mic" className="text-center space-y-5">
      <div className="text-3xl font-sans font-semibold">Microphone</div>
      <p className="text-text-secondary text-base leading-relaxed">
        Inkwell needs access to your microphone to transcribe speech.
        Your audio is processed locally and never sent anywhere.
      </p>
      <div className="text-left space-y-2">
        <label className="text-xs text-text-tertiary uppercase tracking-wider">Input Device</label>
        <select
          value={selectedMic}
          onChange={(e) => {
            setSelectedMic(e.target.value)
            invoke("update_settings", { key: "mic_device", value: e.target.value }).catch((err) => toast(`Could not set microphone: ${err}`))
          }}
          className="w-full px-3 py-2 text-sm rounded-lg border border-border bg-bg-surface text-text-primary focus:outline-none focus:border-text-tertiary"
        >
          <option value="auto">Auto (recommended)</option>
          {micDevices.map((d) => (
            <option key={d.id} value={d.id}>{d.name}</option>
          ))}
        </select>
        {micDevices.length > 0 && (
          <p className="text-body text-text-tertiary">
            {micDevices.length} device{micDevices.length !== 1 ? "s" : ""} detected.
            {selectedMic !== "auto" ? " Restart required to apply." : " Auto skips virtual/AI devices."}
          </p>
        )}
      </div>
    </div>,

    // Step 2: Install a speech model (nothing is bundled, so this is required)
    <div key="model" className="text-center space-y-5">
      <div className="text-3xl font-sans font-semibold">Install a Model</div>
      <p className="text-text-secondary text-base leading-relaxed">
        Inkwell needs a speech model to turn your voice into text. No model is
        installed yet, and it runs entirely on your machine once downloaded.
      </p>
      <div className="bg-bg-surface border border-border rounded-lg p-4 text-left space-y-3">
        <div className="flex justify-between text-sm font-mono">
          <span className="text-text-primary font-medium">Parakeet V3</span>
          <span className="text-accent">670 MB · recommended</span>
        </div>
        <p className="text-xs text-text-tertiary leading-relaxed">
          Smaller models are available in Settings under Models.
        </p>
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
            : "Dictation stays inactive until a model is installed. You can do that anytime in the Models tab."}
        </motion.p>
      )}
    </div>,

    // Step 3 (macOS only): Accessibility permission, before the first dictation test
    ...(IS_MAC ? [
      <div key="accessibility" className="text-center space-y-5">
        <div className="text-3xl font-sans font-semibold">Accessibility</div>
        <p className="text-text-secondary text-base leading-relaxed">
          To type your words into other apps, macOS needs to grant Inkwell
          Accessibility permission. Without it dictation still works, but the
          paste step fails silently and your text only lands on the clipboard.
        </p>
        <div className="bg-bg-surface border border-border rounded-lg p-4 space-y-3">
          <div className="flex items-center justify-between">
            <span className="text-sm text-text-secondary">Permission</span>
            <span className={`text-sm font-mono ${
              accessibility === "granted" ? "text-green-400"
              : accessibility === "denied" ? "text-amber-400"
              : "text-text-tertiary"
            }`}>
              {accessibility === "granted" ? "Granted"
                : accessibility === "denied" ? "Not granted"
                : "Unknown"}
            </span>
          </div>
          {accessibility !== "granted" && (
            <div className="flex justify-center gap-3">
              <button
                onClick={() => { invoke("open_accessibility_settings").catch((e) => toast(`Could not open System Settings: ${e}`, "warning")) }}
                className="px-5 py-2 text-sm font-medium bg-accent text-white rounded-lg hover:bg-accent/90 transition-colors"
              >
                Open System Settings
              </button>
              <button
                onClick={checkAccessibility}
                disabled={accessChecking}
                className="px-4 py-2 text-sm text-text-tertiary hover:text-text-secondary transition-colors disabled:opacity-40"
              >
                {accessChecking ? "Checking..." : "Re-check"}
              </button>
            </div>
          )}
          <p className="text-body text-text-tertiary leading-relaxed">
            Enable Inkwell under Privacy &amp; Security &rarr; Accessibility, then
            hit Re-check. No restart needed. You can skip this and do it later.
          </p>
        </div>
      </div>,
    ] : []),

    // Step 4: Hotkey test
    <div key="hotkey" className="text-center space-y-5">
      <div className="text-3xl font-sans font-semibold">Try It</div>
      <p className="text-text-secondary text-base leading-relaxed">
        Hold <span className="font-mono bg-bg-surface px-2.5 py-1 rounded-md border border-border text-sm">{hotkeyLabel || "your hotkey"}</span> and say something.
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

    // Step 5: Done
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
  // Gate by step identity, not index: the accessibility step only exists on macOS.
  const currentKey = String(steps[step].key)
  const canProceed =
    currentKey === "model" ? (downloadState === "done" || downloadState === "skipped") :
    currentKey === "hotkey" ? hotkeyTested :
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

function Sidebar({
  tabs,
  activeTab,
  onTabChange,
}: {
  tabs: readonly string[]
  activeTab: string
  onTabChange: (t: Tab) => void
}) {
  // Only groups with at least one visible tab render, so basic mode collapses
  // to the handful of items it should show instead of swapping the whole set.
  const groups = tabGroups
    .map((g) => ({ label: g.label, tabs: g.tabs.filter((t) => tabs.includes(t)) }))
    .filter((g) => g.tabs.length > 0)

  const flat = groups.flatMap((g) => g.tabs)

  return (
    <nav
      role="tablist"
      aria-orientation="vertical"
      aria-label="Settings sections"
      className="w-[172px] shrink-0 h-full overflow-y-auto border-r border-border px-2 py-3 space-y-4"
      onKeyDown={(e) => {
        const idx = flat.indexOf(activeTab as Tab)
        if (idx < 0) return
        if (e.key === "ArrowDown" && idx < flat.length - 1) { e.preventDefault(); onTabChange(flat[idx + 1]) }
        if (e.key === "ArrowUp" && idx > 0) { e.preventDefault(); onTabChange(flat[idx - 1]) }
        if (e.key === "Home") { e.preventDefault(); onTabChange(flat[0]) }
        if (e.key === "End") { e.preventDefault(); onTabChange(flat[flat.length - 1]) }
      }}
    >
      {groups.map((group) => (
        <div key={group.label} className="space-y-0.5">
          <p className="px-2.5 pb-1 text-meta font-mono uppercase tracking-wider text-text-tertiary/70">
            {group.label}
          </p>
          {group.tabs.map((tab) => (
            <button
              key={tab}
              role="tab"
              aria-selected={activeTab === tab}
              tabIndex={activeTab === tab ? 0 : -1}
              onClick={() => onTabChange(tab)}
              className={`relative w-full text-left px-2.5 py-1.5 rounded-md text-body transition-colors focus:outline-none focus-visible:ring-1 focus-visible:ring-accent ${
                activeTab === tab
                  ? "text-text-primary bg-bg-hover"
                  : "text-text-secondary hover:text-text-primary hover:bg-bg-hover/50"
              }`}
            >
              {activeTab === tab && (
                <motion.span
                  layoutId="tab-indicator"
                  className="absolute left-0 top-1 bottom-1 w-[2px] bg-accent rounded-full"
                  transition={{ type: "spring", stiffness: 400, damping: 35 }}
                />
              )}
              {tab}
            </button>
          ))}
        </div>
      ))}
    </nav>
  )
}

// --- Tab Router ---

function TabContent({ tab, onAdvancedChange, onNavigate }: { tab: Tab; onAdvancedChange?: (v: boolean) => void; onNavigate?: (t: Tab) => void }) {
  switch (tab) {
    case "Dashboard":   return <DashboardTab />
    case "General":     return <GeneralTab onAdvancedChange={onAdvancedChange} onNavigate={onNavigate} />
    case "Audio":       return <AudioTab />
    case "Models":      return <ModelsTab />
    case "AI":          return <AITab />
    case "Snippets":    return <SnippetsTab />
    case "Modes":       return <ModesTab />
    case "Dictionary":  return <DictionaryTab />
    case "Files":       return <FilesTab />
    case "Commands":    return <VoiceCommandsTab />
    case "Troubleshooting": return <TroubleshootingTab />
    case "About":       return <AboutTab />
  }
}

// --- App ---


function App() {
  const [activeTab, setActiveTab] = useState<Tab>("Dashboard")
  const [modelName, setModelName] = useState("No model loaded")
  const [isRecording, setIsRecording] = useState(false)
  const [isPaused, setIsPaused] = useState(false)
  const [advancedMode, setAdvancedMode] = useState(false)
  const toasts = useToasts((s) => s.toasts)
  const dismissToast = useToasts((s) => s.dismiss)
  const addToast = useToasts((s) => s.push)
  const [updateAvailable, setUpdateAvailable] = useState<UpdateInfo | null>(null)
  const [updateDismissed, setUpdateDismissed] = useState(false)
  const [updateProgress, setUpdateProgress] = useState<number | null>(null)
  const [appVersion, setAppVersion] = useState("")
  // Holds the live Update handle so the install button can act on it (not a window global).
  const updateHandle = useRef<Update | null>(null)

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
          updateHandle.current = update
        }
      } catch (e) {
        console.log("Update check skipped:", e)
      }
    }
    const timer = setTimeout(checkUpdate, 5000)
    return () => { cancelled = true; clearTimeout(timer) }
  }, [])

  // Shipped version comes from the bundle, never a hardcoded literal
  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => {})
  }, [])

  // First-run onboarding
  const [showOnboarding, setShowOnboarding] = useState(false)
  useEffect(() => {
    invoke<boolean>("check_first_run").then((first) => {
      if (first) setShowOnboarding(true)
    }).catch(() => {})
  }, [])

  // Error event listeners. These use the module-level toast() rather than the
  // component-bound push: the store is the same either way, and a plain module
  // function has no place in a dependency list, so the effect can declare an
  // empty one truthfully.
  useEffect(() => {
    const listeners = [
      listen<string>("transcription-error", (e) => toast(`Transcription failed: ${e.payload}`)),
      listen<string>("paste-error", (e) => toast(`Paste failed. Text copied to clipboard: ${e.payload}`, "warning")),
      listen<string>("mic-error", (e) => toast(`Mic error: ${e.payload}`, "warning")),
      listen<string>("model-error", (e) => toast(`Model error: ${e.payload}`, "warning")),
      // Without VAD the recognizer still runs, just unsegmented. Say so rather
      // than degrading silently, which is how this went unnoticed before.
      listen<string>("vad-unavailable", (e) => toast(e.payload, "warning")),
      // Voice editing replaces text the user can see, so both outcomes are
      // reported: silence after speaking an instruction is indistinguishable
      // from the feature not existing.
      listen<string>("voice-edit-error", (e) => toast(e.payload, "warning")),
      listen<string>("voice-edit-done", () => toast("Selection rewritten.", "info")),
    ]
    return () => { listeners.forEach((p) => p.then((fn) => fn())) }
  }, [])

  // Tray "Settings" emits this after focusing the window; land on the settings view.
  useEffect(() => {
    const unlisten = listen("open-settings", () => {
      setShowOnboarding(false)
      setActiveTab("General")
    })
    return () => { unlisten.then((fn) => fn()) }
  }, [])

  // Hydrate the settings store once for the whole app. Every panel reads from
  // it, so this is the only get_settings call in the settings path.
  const loadSettings = useSettings((s) => s.load)
  const storedAdvanced = useSettings((s) => s.settings?.advanced_mode)
  useEffect(() => {
    void loadSettings()
  }, [loadSettings])

  // Resolve the theme choice to a concrete attribute, and keep following the OS
  // while the choice is "system". Re-runs on change so switching away from
  // system detaches the listener rather than stacking another one.
  const themeChoice = (useSettings((s) => s.settings?.theme) ?? "system") as ThemeChoice
  useEffect(() => watchTheme(themeChoice), [themeChoice])
  useEffect(() => {
    if (storedAdvanced !== undefined) setAdvancedMode(storedAdvanced)
  }, [storedAdvanced])

  const tabs = advancedMode ? advancedTabs : basicTabs

  // Reset the tab when a mode change hides the one that is open. Written with
  // the functional setter so the effect can depend on `tabs` honestly instead
  // of listing only advancedMode and lying about what it reads.
  useEffect(() => {
    setActiveTab((current) =>
      (tabs as readonly string[]).includes(current) ? current : "Dashboard",
    )
  }, [tabs])

  // Model name tracking
  useEffect(() => {
    invoke<string>("get_model_name").then(setModelName).catch(() => {})
    const subs = [
      listen<string>("model-loaded", (event) => setModelName(event.payload)),
      listen<boolean>("recording-state", (e) => {
        setIsRecording(e.payload)
        // Pause cannot outlive the session that owns it.
        if (!e.payload) setIsPaused(false)
      }),
      listen<boolean>("recording-paused", (e) => setIsPaused(e.payload)),
    ]
    return () => { subs.forEach((p) => p.then((fn) => fn())) }
  }, [])

  return (
    <div className="h-screen flex bg-bg-base">
      {/* Left: Ink Zone */}
      {/* The ink column was a flat 35% of every view at every width, so on a wide
          window a third of the app was ornament while the content scrolled in
          the remaining two thirds. It now has a ceiling: it still reads as the
          brand at small sizes, and stops growing once it has made its point. */}
      <div className="w-[35%] min-w-[220px] max-w-[380px] h-full relative bg-ink-bg flex flex-col overflow-visible">
        <div className="flex-1 relative overflow-hidden">
          <InkCanvas />
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
                        <p className="text-meta font-mono text-emerald-400">v{updateAvailable.version}</p>
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
                      <p className="text-meta font-mono text-text-tertiary text-right">{updateProgress}%</p>
                    </div>
                  )}
                  {updateProgress === null && (
                    <div className="flex items-center gap-2 pt-0.5">
                      <button
                        onClick={async () => {
                          const update = updateHandle.current
                          if (!update) return
                          setUpdateProgress(0)
                          try {
                            let downloaded = 0
                            // Total only arrives with the Started event; stay at 0% until it does.
                            let total = 0
                            await update.downloadAndInstall((event) => {
                              if (event.event === "Started") {
                                total = event.data.contentLength ?? 0
                              } else if (event.event === "Progress") {
                                downloaded += event.data.chunkLength
                                if (total > 0) setUpdateProgress(Math.min(Math.round((downloaded / total) * 100), 99))
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
        <div className="flex-1 flex min-h-0">
          <Sidebar tabs={tabs} activeTab={activeTab} onTabChange={setActiveTab} />

          <div role="tabpanel" className="flex-1 overflow-y-auto px-6 py-5">
          <AnimatePresence mode="wait">
            <motion.div
              key={activeTab}
              initial={{ opacity: 0, y: 4 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.12 }}
            >
              <TabContent tab={activeTab} onAdvancedChange={setAdvancedMode} onNavigate={setActiveTab} />
            </motion.div>
          </AnimatePresence>
          </div>
        </div>

        {/* Status Bar.
            Recording is the state a user most needs to see and the app never
            showed it here, even though the event was already being listened to
            for the ink canvas. --color-accent-recording had been defined since
            the first build and used nowhere; this is what it was for. */}
        <div className="flex items-center justify-between px-5 py-2.5 border-t border-border">
          <span className="text-meta font-mono text-text-tertiary tracking-wide flex items-center gap-1.5">
            {isRecording ? (
              <>
                <span
                  className={`inline-block w-1.5 h-1.5 rounded-full ${isPaused ? "" : "animate-pulse"}`}
                  style={{ background: isPaused ? "var(--color-text-tertiary)" : "var(--color-accent-recording)" }}
                />
                <span style={{ color: isPaused ? "var(--color-text-tertiary)" : "var(--color-accent-recording)" }}>
                  {isPaused ? "Paused" : "Recording"}
                </span>
              </>
            ) : modelName === "No model loaded" ? (
              <><span className="inline-block w-1.5 h-1.5 rounded-full bg-amber-400" />{modelName}</>
            ) : (
              <><span className="inline-block w-1.5 h-1.5 rounded-full bg-green-400" />{modelName}</>
            )}
          </span>
          <span className="text-meta font-mono text-text-tertiary">{appVersion ? `v${appVersion}` : ""}</span>
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
              onClick={() => dismissToast(toast.id)}
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
