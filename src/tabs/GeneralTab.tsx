import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { SettingRow, GlassToggle, GlassSelect, GlassButton } from "../components/ui"
import type { Settings } from "../types"

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

export function GeneralTab({ onAdvancedChange }: { onAdvancedChange?: (v: boolean) => void }) {
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
