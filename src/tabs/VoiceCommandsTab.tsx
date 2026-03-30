import { useState, useEffect } from "react"
import { motion, AnimatePresence } from "framer-motion"
import { listen } from "@tauri-apps/api/event"
import { invoke } from "@tauri-apps/api/core"
import { GlassToggle } from "../components/ui"
import type { VoiceCommandItem, VoiceCommandStoreData } from "../types"

export function VoiceCommandsTab() {
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
