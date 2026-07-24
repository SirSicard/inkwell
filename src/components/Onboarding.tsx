import { useState, useEffect } from "react"
import { motion } from "framer-motion"
import { listen } from "@tauri-apps/api/event"
import { invoke } from "@tauri-apps/api/core"

export function Onboarding({ onComplete }: { onComplete: () => void }) {
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
