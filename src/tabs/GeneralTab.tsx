import { useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { SettingRow, InkToggle, InkSelect, InkButton } from "../components/ui"
import { useSettings } from "../state/settings"
import { formatHotkey } from "../hotkey"

function HotkeyCapture({
  label = "Hotkey",
  field = "hotkey",
  command = "set_hotkey",
  hint,
  clearable = false,
}: {
  label?: string
  field?: "hotkey" | "edit_hotkey"
  command?: "set_hotkey" | "set_edit_hotkey"
  hint?: string
  clearable?: boolean
}) {
  // The hotkey lives in the shared store; the setter is a separate command
  // because it re-registers the global shortcut, so it is invoked directly and
  // the store is corrected to match on success.
  const hotkey = useSettings((s) => s.settings?.[field] ?? "")
  const [capturing, setCapturing] = useState(false)
  const [error, setError] = useState("")

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

    invoke(command, { hotkey: combo })
      .then(() => {
        void useSettings.getState().load()
        setError("")
      })
      .catch((err) => {
        setError(String(err))
      })
  }

  const clear = () => {
    invoke(command, { hotkey: "" })
      .then(() => { void useSettings.getState().load(); setError("") })
      .catch((err) => setError(String(err)))
  }

  const displayHotkey = capturing
    ? "Press your hotkey..."
    : hotkey
      ? formatHotkey(hotkey)
      : "Disabled"

  return (
    <div className="space-y-1">
      <label className="text-xs text-text-tertiary uppercase tracking-wider">{label}</label>
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
      {hint && <p className="text-body text-text-tertiary">{hint}</p>}
      {clearable && hotkey && (
        <button
          onClick={clear}
          className="text-body text-text-tertiary hover:text-text-primary transition-colors"
        >
          Turn off
        </button>
      )}
      {error && <p className="text-xs text-red-400">{error}</p>}
    </div>
  )
}

export function GeneralTab({ onAdvancedChange, onNavigate }: { onAdvancedChange?: (v: boolean) => void; onNavigate?: (tab: "Modes" | "Troubleshooting") => void }) {
  // Read straight from the store rather than mirroring it into local state:
  // the mirrors were the reason two panels could disagree about the same value.
  const settings = useSettings((s) => s.settings)
  const setSetting = useSettings((s) => s.set)

  const startOnBoot = settings?.start_on_boot ?? false
  const showOverlay = settings?.show_overlay ?? true
  const overlayPosition = settings?.overlay_position ?? "bottom-center"
  const theme = settings?.theme ?? "system"
  const recordingMode = settings?.recording_mode ?? "ptt"
  const advancedMode = settings?.advanced_mode ?? false
  const soundDictation = settings?.sound_dictation ?? true
  
  const handleRecordingModeChange = (value: string) => setSetting("recording_mode", value)
  const handleStartOnBootChange = (value: boolean) => setSetting("start_on_boot", value)
  const handleShowOverlayChange = (value: boolean) => setSetting("show_overlay", value)

  const handleAdvancedModeChange = (value: boolean) => {
    void setSetting("advanced_mode", value)
    onAdvancedChange?.(value)
  }

  return (
    <div className="space-y-3">
      <h2 className="text-heading font-sans font-semibold text-text-primary">General</h2>

      <SettingRow label="Recording Mode" description="How the hotkey behaves">
        <InkSelect
          value={recordingMode}
          onChange={handleRecordingModeChange}
          options={[
            { label: "Toggle", value: "toggle" },
            { label: "Push to Talk", value: "ptt" },
          ]}
        />
      </SettingRow>

      {/* Text style and speech cleanup both used to be set here, writing
          settings.style and settings.remove_fillers. The pipeline now resolves
          both from the active mode, so these controls had become switches that
          silently did nothing: exactly the "two places to set one thing" problem
          modes existed to remove. They are edited where they are read. */}
      <SettingRow
        label="Text Style and Cleanup"
        description="Formatting, speech cleanup and polish belong to a mode now, so they can differ per app."
      >
        <InkButton onClick={() => onNavigate?.("Modes")}>Open Modes</InkButton>
      </SettingRow>

      <SettingRow label="Appearance" description="Follow the system, or pick one">
        <InkSelect
          value={theme}
          onChange={(v) => setSetting("theme", v)}
          options={[
            { label: "System", value: "system" },
            { label: "Light", value: "light" },
            { label: "Dark", value: "dark" },
          ]}
        />
      </SettingRow>

      <SettingRow label="Start on Boot" description="Launch Inkwell when you log in">
        <InkToggle checked={startOnBoot} onChange={handleStartOnBootChange} />
      </SettingRow>

      <SettingRow label="Show Overlay" description="Floating indicator while recording">
        <InkToggle checked={showOverlay} onChange={handleShowOverlayChange} />
      </SettingRow>

      {showOverlay && (
        <SettingRow label="Overlay Position" description="Where the indicator sits while you dictate">
          <InkSelect
            value={overlayPosition}
            onChange={(v) => setSetting("overlay_position", v)}
            options={[
              { label: "Top left", value: "top-left" },
              { label: "Top center", value: "top-center" },
              { label: "Top right", value: "top-right" },
              { label: "Bottom left", value: "bottom-left" },
              { label: "Bottom center", value: "bottom-center" },
              { label: "Bottom right", value: "bottom-right" },
            ]}
          />
        </SettingRow>
      )}

      <SettingRow label="Dictation Sound" description="Audio feedback when recording starts and stops">
        <InkToggle checked={soundDictation} onChange={(v) => setSetting("sound_dictation", v)} />
      </SettingRow>

      <SettingRow label="Advanced Mode" description="Show all tabs and settings">
        <InkToggle checked={advancedMode} onChange={handleAdvancedModeChange} />
      </SettingRow>

      {/* Debug audio recording used to sit here behind Advanced Mode. It is a
          diagnostic you switch on for an hour, not a preference you set once,
          and it writes voice to disk while on, so it lives with the other
          diagnostics instead of beside the theme picker. */}
      {advancedMode && (
        <SettingRow
          label="Diagnostics"
          description="Debug recording and transcription troubleshooting."
        >
          <InkButton onClick={() => onNavigate?.("Troubleshooting")}>Open Troubleshooting</InkButton>
        </SettingRow>
      )}


      <HotkeyCapture />

      <HotkeyCapture
        label="Voice Edit Hotkey"
        field="edit_hotkey"
        command="set_edit_hotkey"
        clearable
        hint="Select text anywhere, hold this, and say what to change. Needs an API key, since rewriting is the model's job."
      />

      <div className="flex gap-2 pt-2">
        <InkButton variant="ghost" onClick={() => {
          void setSetting("style", "formal")
          void setSetting("recording_mode", "ptt")
          void setSetting("show_overlay", true)
          void setSetting("start_on_boot", false)
          window.location.reload()
        }}>Reset Defaults</InkButton>
      </div>
    </div>
  )
}
