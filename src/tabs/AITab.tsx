import { useState, useEffect } from "react"
import { motion, AnimatePresence } from "framer-motion"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "../state/toasts"
import { GlassToggle } from "../components/ui"

function AIConsentModal({ onAccept, onDecline }: { onAccept: () => void; onDecline: () => void }) {
  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-6"
    >
      <motion.div
        initial={{ opacity: 0, y: 24, scale: 0.97 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        exit={{ opacity: 0, y: 12 }}
        transition={{ type: "spring", stiffness: 300, damping: 28 }}
        className="max-w-md w-full bg-bg-base border border-border rounded-xl p-6 space-y-5"
      >
        <div className="space-y-1">
          <h3 className="text-base font-sans font-medium text-text-primary">Before you enable AI Polish</h3>
          <p className="text-xs text-text-tertiary">A quick heads-up about how this works.</p>
        </div>

        <div className="space-y-3 text-sm text-text-secondary">
          <p>AI Polish sends your transcription text to an LLM to clean up grammar, punctuation, and capitalization.</p>
          <p className="text-text-tertiary text-xs">Your audio never leaves your device. Only the final text is sent.</p>
        </div>

        <div className="bg-bg-surface border border-border rounded-lg p-4 space-y-2">
          <p className="text-sm text-text-primary font-medium">Bring your own key</p>
          <p className="text-xs text-text-tertiary">
            Requests go straight from your machine to the provider you configure, using your
            own API key. There is no Inkwell server in the path, no account, and no quota.
          </p>
        </div>

        <div className="flex gap-3 pt-1">
          <button onClick={onAccept} className="flex-1 px-4 py-2.5 text-sm bg-accent text-white rounded-lg hover:bg-accent/90 transition-colors font-medium">
            Enable AI Polish
          </button>
          <button onClick={onDecline} className="px-4 py-2.5 text-sm text-text-tertiary hover:text-text-secondary transition-colors">
            Not now
          </button>
        </div>
      </motion.div>
    </motion.div>
  )
}

export function AITab() {
  const [provider, setProvider] = useState("groq")
  const [apiKey, setApiKey] = useState("")
  const [keyStatus, setKeyStatus] = useState<Record<string, boolean>>({})
  const [polishEnabled, setPolishEnabled] = useState(false)
  const [showConsent, setShowConsent] = useState(false)
  const [polishPrompt, setPolishPrompt] = useState("")
  const [saving, setSaving] = useState(false)
  const [testText, setTestText] = useState("")
  const [testResult, setTestResult] = useState("")
  const [testing, setTesting] = useState(false)
  const [showPrompt, setShowPrompt] = useState(false)
  const [showTest, setShowTest] = useState(false)

  const providers = [
    { id: "groq", label: "Groq", hint: "llama-3.3-70b", freeKey: true },
    { id: "openai", label: "OpenAI", hint: "gpt-4o-mini" },
    { id: "anthropic", label: "Anthropic", hint: "Claude Haiku" },
    { id: "openrouter", label: "OpenRouter", hint: "any model" },
  ] as const

  const hasAnyKey = Object.values(keyStatus).some(Boolean)

  // Mirrors llm::PROVIDERS in src-tauri/src/llm.rs: with no explicit provider the
  // backend polishes with the first key it finds, in exactly this order.
  const providerOrder = ["openai", "groq", "anthropic", "openrouter", "custom"]
  const activeProvider = keyStatus[provider]
    ? provider
    : providerOrder.find((p) => keyStatus[p]) ?? null
  const activeLabel = providers.find((p) => p.id === activeProvider)?.label ?? activeProvider

  const handleTogglePolish = (enabled: boolean) => {
    if (enabled && !polishEnabled) {
      setShowConsent(true)
    } else {
      setPolishEnabled(false)
      invoke("set_polish_settings", { enabled: false, prompt: polishPrompt }).catch((e) => toast(`Could not disable polish: ${e}`))
    }
  }

  const handleConsentAccept = () => {
    setShowConsent(false)
    setPolishEnabled(true)
    invoke("set_polish_settings", { enabled: true, prompt: polishPrompt }).catch((e) => toast(`Could not enable polish: ${e}`))
  }

  useEffect(() => {
    invoke<Record<string, boolean>>("get_api_key_status").then(setKeyStatus).catch((e) => toast(`Could not read key status: ${e}`, "warning"))
    invoke<{ enabled: boolean; prompt: string }>("get_polish_settings").then((s) => {
      setPolishEnabled(s.enabled)
      setPolishPrompt(s.prompt)
    }).catch(() => {})
  }, [])

  const handleSaveKey = async () => {
    setSaving(true)
    try {
      await invoke("save_api_key", { provider, key: apiKey })
      const status = await invoke<Record<string, boolean>>("get_api_key_status")
      setKeyStatus(status)
      setApiKey("")
    } catch (e) { console.error(e) }
    setSaving(false)
  }

  const handleSavePolish = () => {
    invoke("set_polish_settings", { enabled: polishEnabled, prompt: polishPrompt }).catch((e) => toast(`Could not save polish settings: ${e}`))
  }

  const handleTest = async () => {
    if (!testText.trim()) return
    setTesting(true)
    setTestResult("")
    try {
      // Omitting the provider lets the backend pick the first configured key.
      const useProvider = keyStatus[provider] ? provider : null
      const result = await invoke<{ text: string }>(
        "run_ai_polish", { text: testText, provider: useProvider, model: null }
      )
      setTestResult(result.text)
    } catch (e) {
      setTestResult(`Error: ${e}`)
    }
    setTesting(false)
  }

  return (
    <div className="space-y-4">
      {/* Consent modal */}
      <AnimatePresence>
        {showConsent && (
          <AIConsentModal
            onAccept={handleConsentAccept}
            onDecline={() => setShowConsent(false)}
          />
        )}
      </AnimatePresence>

      {/* -- Section 1: Master toggle -- */}
      <div className="p-4 bg-bg-surface border border-border rounded-xl space-y-3">
        <div className="flex items-center justify-between">
          <div>
            <p className="text-sm font-medium text-text-primary">AI Polish</p>
            <p className="text-xs text-text-tertiary mt-0.5">
              {polishEnabled
                ? hasAnyKey
                  ? `Polishing with your ${activeLabel} key`
                  : "No API key configured"
                : "Off. Transcriptions are raw."
              }
            </p>
          </div>
          <GlassToggle checked={polishEnabled} onChange={handleTogglePolish} />
        </div>

        {/* Polish is BYOK-only: enabled without a key does nothing until one is added. */}
        {polishEnabled && !hasAnyKey && (
          <div className="rounded-lg border border-amber-800/30 bg-amber-950/20 px-3 py-2.5">
            <p className="text-xs text-amber-200">No API key configured.</p>
            <p className="text-[11px] text-amber-200/70 mt-0.5">
              Add one under API Keys below. Until then transcriptions stay raw.
            </p>
          </div>
        )}
      </div>

      {/* -- Section 2: API Keys -- */}
      <div className="p-4 bg-bg-surface border border-border rounded-xl space-y-3">
        <div>
          <p className="text-sm font-medium text-text-primary">API Keys</p>
          <p className="text-xs text-text-tertiary mt-0.5">
            {hasAnyKey ? "Your key, direct to the model. Stored in your OS keyring." : "AI Polish needs your own key. It never touches our servers."}
          </p>
        </div>

        {/* Provider row */}
        <div className="flex gap-1.5 flex-wrap">
          {providers.map((p) => (
            <button
              key={p.id}
              onClick={() => setProvider(p.id)}
              className={`flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border transition-all ${
                provider === p.id
                  ? "border-text-tertiary/40 text-text-primary bg-bg-base"
                  : "border-transparent text-text-tertiary hover:text-text-secondary"
              }`}
            >
              {p.label}
              {"freeKey" in p && p.freeKey && !keyStatus[p.id] && (
                <span className="text-[10px] px-1 py-0.5 rounded bg-green-400/10 text-green-400">free key</span>
              )}
              {keyStatus[p.id] && (
                <span className="w-1.5 h-1.5 rounded-full bg-green-400" title="Configured" />
              )}
            </button>
          ))}
        </div>

        {/* Key input */}
        <div className="flex gap-2">
          <input
            type="password"
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && apiKey.trim() && handleSaveKey()}
            placeholder={keyStatus[provider] ? `${providers.find(p => p.id === provider)?.label} key saved` : `Paste ${providers.find(p => p.id === provider)?.label} API key...`}
            className={`flex-1 px-3 py-2 text-sm bg-bg-base border rounded-lg text-text-primary placeholder:text-text-tertiary focus:outline-none transition-colors ${
              keyStatus[provider] ? "border-green-800/30" : "border-border focus:border-border-default"
            }`}
          />
          {apiKey.trim() ? (
            <button
              onClick={handleSaveKey}
              disabled={saving}
              className="px-4 py-2 text-xs font-mono bg-accent text-white rounded-lg hover:bg-accent/90 transition-colors disabled:opacity-40"
            >
              {saving ? "..." : "Save"}
            </button>
          ) : keyStatus[provider] ? (
            <button
              onClick={() => { invoke("save_api_key", { provider, key: "" }).then(() => invoke<Record<string, boolean>>("get_api_key_status").then(setKeyStatus)) }}
              className="px-3 py-2 text-xs font-mono text-red-400/70 hover:text-red-400 transition-colors"
            >
              Remove
            </button>
          ) : null}
        </div>

        <p className="text-[10px] text-text-tertiary">
          {providers.find(p => p.id === provider)?.hint}
          {provider === "groq" && !keyStatus.groq && " \u00b7 get a free key at console.groq.com"}
        </p>
      </div>

      {/* -- Section 3: Collapsible prompt editor -- */}
      <div className="border border-border rounded-xl overflow-hidden">
        <button
          onClick={() => setShowPrompt(!showPrompt)}
          className="w-full flex items-center justify-between px-4 py-3 text-left hover:bg-bg-surface/80 transition-colors"
        >
          <div>
            <p className="text-sm text-text-secondary">Polish Prompt</p>
            <p className="text-[10px] text-text-tertiary">How the LLM processes your text</p>
          </div>
          <motion.span
            animate={{ rotate: showPrompt ? 180 : 0 }}
            className="text-text-tertiary text-xs"
          >
            &#9662;
          </motion.span>
        </button>
        <AnimatePresence>
          {showPrompt && (
            <motion.div
              initial={{ height: 0, opacity: 0 }}
              animate={{ height: "auto", opacity: 1 }}
              exit={{ height: 0, opacity: 0 }}
              transition={{ duration: 0.2 }}
              className="overflow-hidden"
            >
              <div className="px-4 pb-4 space-y-2">
                <textarea
                  value={polishPrompt}
                  onChange={(e) => setPolishPrompt(e.target.value)}
                  onBlur={handleSavePolish}
                  rows={3}
                  className="w-full px-3 py-2 text-xs bg-bg-base border border-border rounded-lg text-text-secondary focus:outline-none focus:border-border-default resize-none font-mono leading-relaxed"
                />
              </div>
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      {/* -- Section 4: Collapsible test panel -- */}
      <div className="border border-border rounded-xl overflow-hidden">
        <button
          onClick={() => setShowTest(!showTest)}
          className="w-full flex items-center justify-between px-4 py-3 text-left hover:bg-bg-surface/80 transition-colors"
        >
          <div>
            <p className="text-sm text-text-secondary">Test</p>
            <p className="text-[10px] text-text-tertiary">Try AI Polish on sample text</p>
          </div>
          <motion.span
            animate={{ rotate: showTest ? 180 : 0 }}
            className="text-text-tertiary text-xs"
          >
            &#9662;
          </motion.span>
        </button>
        <AnimatePresence>
          {showTest && (
            <motion.div
              initial={{ height: 0, opacity: 0 }}
              animate={{ height: "auto", opacity: 1 }}
              exit={{ height: 0, opacity: 0 }}
              transition={{ duration: 0.2 }}
              className="overflow-hidden"
            >
              <div className="px-4 pb-4 space-y-2">
                <textarea
                  value={testText}
                  onChange={(e) => setTestText(e.target.value)}
                  placeholder="Paste raw transcription text..."
                  rows={2}
                  className="w-full px-3 py-2 text-xs bg-bg-base border border-border rounded-lg text-text-secondary placeholder:text-text-tertiary focus:outline-none focus:border-border-default resize-none"
                />
                <div className="flex items-center gap-2">
                  <button
                    onClick={handleTest}
                    disabled={testing || !testText.trim()}
                    className="px-4 py-1.5 text-xs font-mono bg-bg-base border border-border text-text-secondary hover:text-text-primary rounded-lg transition-colors disabled:opacity-40"
                  >
                    {testing ? "Polishing..." : "Run"}
                  </button>
                  {testResult && !testing && activeProvider && (
                    <span className="text-[10px] text-text-tertiary">via {activeProvider}</span>
                  )}
                </div>
                {testResult && (
                  <motion.div
                    initial={{ opacity: 0, y: 4 }}
                    animate={{ opacity: 1, y: 0 }}
                    className="p-3 bg-bg-base border border-border rounded-lg"
                  >
                    <p className="text-sm text-text-primary leading-relaxed">{testResult}</p>
                  </motion.div>
                )}
              </div>
            </motion.div>
          )}
        </AnimatePresence>
      </div>
    </div>
  )
}
