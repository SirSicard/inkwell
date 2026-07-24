import { useState, useEffect } from "react"
import { getVersion } from "@tauri-apps/api/app"
import { DONATION_URL } from "../constants"

export function AboutTab() {
  const [appVersion, setAppVersion] = useState("")

  // Shipped version comes from the bundle, never a hardcoded literal
  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => {})
  }, [])

  return (
    <div className="flex items-center justify-center h-full min-h-[400px]">
      <div className="text-center space-y-4">
        <div className="text-4xl font-sans font-bold tracking-tight text-text-primary">INKWELL</div>
        <p className="text-base text-text-secondary">Your voice. Your machine. Nothing leaves.</p>
        <div className="pt-3 space-y-1.5">
          <p className="text-sm text-text-tertiary">Built by Mattias H.</p>
          <p className="text-[11px] font-mono text-text-tertiary">{appVersion ? `v${appVersion}` : ""}</p>
        </div>
        <div className="pt-4 flex items-center justify-center gap-4">
          <span className="text-xs text-text-tertiary">Tauri + React + sherpa-onnx</span>
        </div>
        <div className="pt-2 space-y-1.5">
          <p className="text-[11px] text-text-tertiary">
            Free and open source. No accounts, no telemetry, no paid tier.
          </p>
          <p className="text-[11px] text-text-tertiary">
            <a href={DONATION_URL} target="_blank" rel="noopener noreferrer" className="underline hover:text-text-secondary transition-colors">
              Support Inkwell
            </a>
            {" "}if it saves you time
          </p>
        </div>
        <div className="pt-2">
          <p className="text-[11px] text-text-tertiary">
            Built on{" "}
            <a href="https://github.com/cjpais/Handy" target="_blank" rel="noopener noreferrer" className="underline hover:text-text-secondary transition-colors">
              Handy
            </a>
            {" "}by CJ Pais
          </p>
        </div>
      </div>
    </div>
  )
}
