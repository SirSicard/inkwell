import { useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { SettingRow, InkToggle, InkButton } from "../components/ui"
import { useSettings } from "../state/settings"
import { toast } from "../state/toasts"

/**
 * Diagnostics, kept out of General.
 *
 * Save Debug Audio lived in General behind Advanced Mode, next to the hotkey
 * and the theme. It is not a preference: it is a tool you switch on to
 * investigate a problem and switch off again, and it writes your voice to disk
 * while it is on. Settings people set once and settings people use for an hour
 * do not belong in the same list.
 */
export function TroubleshootingTab() {
  const settings = useSettings((s) => s.settings)
  const setSetting = useSettings((s) => s.set)
  const debugSaveAudio = settings?.debug_save_audio ?? false
  const [folder, setFolder] = useState("")
  const [logFolder, setLogFolder] = useState("")

  const openFolder = () => {
    invoke<string>("open_debug_audio_folder")
      .then(setFolder)
      .catch((e) => toast(`Could not open the folder: ${e}`))
  }

  const openLogFolder = () => {
    invoke<string>("open_log_folder")
      .then(setLogFolder)
      .catch((e) => toast(`Could not open the log folder: ${e}`))
  }

  return (
    <div className="space-y-3">
      <h2 className="text-heading font-sans font-semibold text-text-primary">Troubleshooting</h2>

      <SettingRow
        label="Save Debug Audio"
        description="Writes every dictation to disk as a WAV so a recording can be examined or compared between models. Your voice stays on your machine, but it does stay: turn this off when you are done and delete what you no longer need."
      >
        <InkToggle checked={debugSaveAudio} onChange={(v) => setSetting("debug_save_audio", v)} />
      </SettingRow>

      {debugSaveAudio && (
        <div className="rounded-lg border border-border bg-bg-surface p-3.5 space-y-2">
          <p className="text-body text-text-secondary">
            Recordings are saved as <span className="font-mono text-text-primary">take-0001.wav</span> and
            upward in your Documents folder. To measure which model hears you best, write what you
            actually said into a matching <span className="font-mono text-text-primary">take-0001.txt</span> beside
            each one, then run the comparison tool from the repository.
          </p>
          <div className="flex items-center gap-2">
            <InkButton variant="ghost" onClick={openFolder}>Open recordings folder</InkButton>
            {folder && <span className="text-meta font-mono text-text-tertiary truncate">{folder}</span>}
          </div>
        </div>
      )}

      <SettingRow
        label="Log"
        description="A running record of what the app did: which model loaded, how long each recording was, how many characters each stage produced, and anything that failed. Your dictated text is not written to it, only its length, so the log can be attached to a bug report without sending your words along with it."
      >
        <InkButton variant="ghost" onClick={openLogFolder}>Open log folder</InkButton>
      </SettingRow>
      {logFolder && (
        <p className="px-3.5 text-meta font-mono text-text-tertiary truncate">{logFolder}</p>
      )}

      <SettingRow
        label="Recording Level"
        description="Dictation is normalised automatically before transcription, so a quiet microphone is not usually the cause of a bad transcript. If words are wrong rather than missing, the Dictionary is the faster fix: entries there bias the recogniser, not just the text it produces."
      >
        <span className="text-meta font-mono text-text-tertiary">automatic</span>
      </SettingRow>
    </div>
  )
}
