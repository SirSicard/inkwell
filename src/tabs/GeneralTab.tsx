import { useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { SettingRow, InkToggle, InkSelect, InkButton } from "../components/ui"
import { useSettings } from "../state/settings"
import type { PartialsStatus } from "../types"
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

    // A global hotkey swallows its key in every app on the machine, so which
    // single keys are allowed is a safety question, not a parsing one. Keys
    // nobody types with can stand alone; a bare letter, digit or space would
    // stop that key working system-wide the moment it saved, which reads as a
    // broken keyboard, not a bad setting. Shift-only counts as bare for
    // typing keys: claiming shift+a is claiming the capital A everywhere.
    const BARE_SAFE = /^(f([1-9]|1[0-9]|2[0-4])|insert|pause|scrolllock)$/
    const keyToken = combo.split("+").pop() ?? ""
    const hasRealModifier = e.ctrlKey || e.altKey || e.metaKey
    if (!hasRealModifier && !BARE_SAFE.test(keyToken)) {
      setError(
        e.shiftKey
          ? "Shift plus a typing key would stop that character working everywhere. Add Ctrl, Alt or Cmd, or use an F-key."
          : "A key you type with cannot stand alone: the hotkey would swallow it in every app. Use an F-key by itself (F1 to F24), or add a modifier.",
      )
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
      {navigator.userAgent.includes("Mac") && (
        <div className="flex flex-wrap items-center gap-1.5 pt-0.5">
          <span className="text-meta text-text-tertiary">Or a key on its own:</span>
          {(
            [
              ["fn", "Fn \u{1F310}"],
              ["right_cmd", "Right \u2318"],
              ["right_opt", "Right \u2325"],
              ["right_ctrl", "Right \u2303"],
            ] as const
          ).map(([token, name]) => (
            <button
              key={token}
              onClick={() => {
                invoke(command, { hotkey: token })
                  .then(() => { void useSettings.getState().load(); setError("") })
                  .catch((err) => setError(String(err)))
              }}
              className={`px-2 py-0.5 text-xs rounded-md border transition-colors ${
                hotkey === token
                  ? "border-accent text-text-primary"
                  : "border-border text-text-secondary hover:text-text-primary hover:border-border-default"
              }`}
            >
              {name}
            </button>
          ))}
        </div>
      )}
      {hotkey === "fn" && (
        <p className="text-body text-text-tertiary">
          macOS gives the globe key its own job by default. Set System Settings
          {" \u2192 "}Keyboard{" \u2192 "}&ldquo;Press {"\u{1F310}"} key
          to&rdquo; to <span className="text-text-secondary">Do Nothing</span>,
          or each dictation will also open the emoji picker.
        </p>
      )}
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

/**
 * Live Preview: words on the overlay while you are still speaking.
 *
 * It has its own row rather than being a plain toggle because it is the only
 * setting in this tab that costs a download. Switching it on with nothing on
 * disk would be a switch that appears to do nothing, so the row asks for the
 * download first and only then offers the toggle.
 */
function LivePreviewRow() {
  const settings = useSettings((s) => s.settings)
  const setSetting = useSettings((s) => s.set)
  const [status, setStatus] = useState<PartialsStatus | null>(null)
  const [percent, setPercent] = useState<number | null>(null)
  const [error, setError] = useState("")

  const refresh = () =>
    invoke<PartialsStatus>("get_partials_status").then(setStatus).catch(() => {})

  useEffect(() => {
    void refresh()
  }, [])

  const download = async () => {
    if (!status) return
    setError("")
    setPercent(0)
    const stop = await listen<{ percent: number; model: string }>(
      "model-download-progress",
      (e) => {
        if (e.payload.model === status.model_id) setPercent(e.payload.percent)
      },
    )
    try {
      await invoke("download_model", { modelId: status.model_id })
      await refresh()
      // Turning it on is what actually loads the model, so the download is
      // only half the job and doing it by hand afterwards is a step nobody
      // would guess at.
      await setSetting("show_partials", true)
    } catch (e) {
      setError(String(e))
    } finally {
      stop()
      setPercent(null)
    }
  }

  const description =
    "Shows words on the overlay as you speak them, so the wait for the paste is not silent. " +
    "They are lowercase and unpunctuated because they come from a second, faster model; " +
    "what gets pasted is unchanged."

  return (
    <SettingRow label="Live Preview" description={description}>
      {percent !== null ? (
        <span className="text-body text-text-tertiary tabular-nums">{percent}%</span>
      ) : status && !status.installed ? (
        <InkButton onClick={download}>Download {status.size}</InkButton>
      ) : (
        <InkToggle
          checked={settings?.show_partials ?? false}
          onChange={(v) => setSetting("show_partials", v)}
        />
      )}
      {error && <p className="text-xs text-red-400">{error}</p>}
    </SettingRow>
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

      <SettingRow
        label="Space After Dictation"
        description="Appends one space after each pasted dictation, so dictating twice in a row does not fuse the last word of one with the first of the next."
      >
        <InkToggle
          checked={settings?.append_space ?? true}
          onChange={(v) => setSetting("append_space", v)}
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

      {showOverlay && <LivePreviewRow />}

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


      <HotkeyCapture
        hint={
          navigator.userAgent.includes("Mac")
            ? "A single F-key works on its own, for example F5. On a Mac keyboard the F-keys may need the Fn key held, unless \u201cUse F1, F2, etc. as standard function keys\u201d is on in System Settings."
            : "A single F-key works on its own, for example F5."
        }
      />

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
