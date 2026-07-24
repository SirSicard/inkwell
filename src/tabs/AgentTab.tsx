import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { GlassCard, GlassToggle, GlassInput, GlassButton, GlassSelect } from "../components/ui"

export function AgentTab() {
  const [token, setToken] = useState("")
  const [hasToken, setHasToken] = useState(false)
  const [testStatus, setTestStatus] = useState<string | null>(null)
  const [testing, setTesting] = useState(false)
  const [settings, setSettings] = useState<any>(null)

  useEffect(() => {
    invoke<boolean>("get_agent_token_status").then(setHasToken).catch(() => {})
    invoke<any>("get_settings").then(setSettings).catch(() => {})
  }, [])

  const updateSetting = async (key: string, value: any) => {
    const updated = { ...settings, [key]: value }
    setSettings(updated)
    try {
      await invoke("update_settings", { settings: updated })
    } catch (e) {
      console.error("Failed to save settings:", e)
    }
  }

  const saveToken = async () => {
    if (!token.trim()) return
    try {
      await invoke("save_agent_token", { token: token.trim() })
      setHasToken(true)
      setToken("")
      setTestStatus("Token saved")
    } catch (e: any) {
      setTestStatus(`Error: ${e}`)
    }
  }

  const testConnection = async () => {
    setTesting(true)
    setTestStatus(null)
    try {
      const result = await invoke<string>("test_agent_connection")
      setTestStatus(result)
    } catch (e: any) {
      setTestStatus(`Error: ${e}`)
    }
    setTesting(false)
  }

  if (!settings) return null

  return (
    <div className="space-y-4">
      <div className="text-xs text-text-secondary uppercase tracking-wider mb-2">
        Voice Agent (OpenClaw)
      </div>

      <GlassCard>
        <div className="flex items-center justify-between">
          <div>
            <div className="text-sm text-text-primary font-medium">Enable Agent Mode</div>
            <div className="text-xs text-text-secondary mt-0.5 leading-relaxed">
              Second hotkey to send voice commands to OpenClaw
            </div>
          </div>
          <GlassToggle
            checked={settings.agent_enabled || false}
            onChange={(v: boolean) => updateSetting("agent_enabled", v)}
          />
        </div>
      </GlassCard>

      {settings.agent_enabled && (
        <>
          <GlassCard>
            <div className="space-y-3">
              <div>
                <div className="text-sm text-text-primary mb-1">Agent Hotkey</div>
                <GlassInput
                  value={settings.agent_hotkey || "ctrl+shift+space"}
                  onChange={(e: any) => updateSetting("agent_hotkey", e.target.value)}
                  placeholder="ctrl+shift+space"
                />
                <div className="text-xs text-text-secondary mt-1 leading-relaxed">
                  Hold to record, release to send to OpenClaw. Restart required after changing.
                </div>
              </div>
            </div>
          </GlassCard>

          <GlassCard>
            <div className="space-y-3">
              <div>
                <div className="text-sm text-text-primary mb-1">Gateway URL</div>
                <GlassInput
                  value={settings.agent_url || "http://127.0.0.1:41738"}
                  onChange={(e: any) => updateSetting("agent_url", e.target.value)}
                  placeholder="http://127.0.0.1:41738"
                />
              </div>
              <div>
                <div className="text-sm text-text-primary mb-1">Agent ID</div>
                <GlassInput
                  value={settings.agent_id || "main"}
                  onChange={(e: any) => updateSetting("agent_id", e.target.value)}
                  placeholder="main"
                />
              </div>
              <div>
                <div className="text-sm text-text-primary mb-1">Model</div>
                <GlassSelect
                  value={settings.agent_model || "sonnet"}
                  onChange={(v: string) => updateSetting("agent_model", v)}
                  options={[
                    { value: "sonnet", label: "Sonnet (fast)" },
                    { value: "opus", label: "Opus (full power)" },
                  ]}
                />
                <div className="text-xs text-text-secondary mt-1 leading-relaxed">
                  Sonnet is faster (2-3s). Opus is smarter but slower (5-15s).
                </div>
              </div>
              <div>
                <div className="text-sm text-text-primary mb-1">
                  Gateway Token {hasToken && <span className="text-green-400 ml-1">saved</span>}
                </div>
                <div className="flex gap-2">
                  <div className="flex-1">
                    <GlassInput
                      value={token}
                      onChange={(e: any) => setToken(e.target.value)}
                      placeholder={hasToken ? "Token saved (enter new to replace)" : "Paste your gateway token"}
                      type="password"
                    />
                  </div>
                  <GlassButton onClick={saveToken} disabled={!token.trim()}>
                    Save
                  </GlassButton>
                </div>
              </div>
              <div className="flex gap-2 items-center">
                <GlassButton onClick={testConnection} disabled={testing || !hasToken}>
                  {testing ? "Testing..." : "Test Connection"}
                </GlassButton>
                {testStatus && (
                  <span className={`text-xs ${testStatus.startsWith("Error") ? "text-red-400" : "text-green-400"}`}>
                    {testStatus}
                  </span>
                )}
              </div>
            </div>
          </GlassCard>

          <div className="text-xs text-text-secondary leading-relaxed px-1">
            Voice agent sends your spoken text to OpenClaw. The response appears in your
            active OpenClaw session (webchat, Telegram, etc). Make sure your gateway is
            running and the chat completions endpoint is enabled in your OpenClaw config.
          </div>
        </>
      )}
    </div>
  )
}
