import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { SettingRow, GlassSelect } from "../components/ui"
import type { Settings, DeviceInfo } from "../types"

export function AudioTab() {
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
