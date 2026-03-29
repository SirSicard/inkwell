# AI Polish / BYOK LLM Integration - Research

## Crate Stack
- **`reqwest`** (with `stream` feature) for HTTP. Not `async-openai` (too OpenAI-specific, Anthropic uses different API shape).
- **`reqwest-eventsource`** for SSE streaming (handles retries, parses event stream)
- **`keyring`** crate for secure API key storage (Windows Credential Manager, macOS Keychain, Linux Secret Service)
- **`handlebars`** for prompt templates
- **`chrono`** for usage tracking week boundaries

## Secure Key Storage
Use `keyring` crate (or `tauri-plugin-keyring`):
- `Entry::new("inkwell", "openai").set_password(&key)` to store
- Keys never leave Rust backend. Frontend only sends "save key for provider X" and gets "configured / not configured" status.
- Masked input fields in settings UI per provider.

## Multi-Provider Abstraction
Key insight: OpenAI, Groq, Cerebras, OpenRouter all use same `/v1/chat/completions` schema. Anthropic is the outlier.

Two implementations:
1. **`OpenAICompatible`**: covers OpenAI, Groq, Cerebras, OpenRouter. Different `base_url`, same request/response shape.
2. **`AnthropicProvider`**: different auth (`x-api-key`), different streaming format (`content_block_delta`).

Trait: `LlmProvider` with `complete()` and `stream()` methods. Each impl handles its own SSE parsing, outputs uniform `Stream<Item = Result<String>>`.

## SSE Streaming
- `reqwest-eventsource` wraps reqwest, gives `Event::Message` with data
- OpenAI: `data: {"choices":[{"delta":{"content":"token"}}]}`
- Anthropic: `event: content_block_delta` with `delta.text`
- Push tokens to frontend via Tauri events (`app.emit("llm-token", token)`)

## Free Tier Design (4K words/week)
- Local word counter in `usage.json` (app data dir)
- Count words from transcription input, not LLM tokens
- Reset every Monday (ISO week boundary)
- Skip counter when BYOK key is configured
- Show usage bar in UI: "1,247 / 4,000 words this week"
- Tampering is fine (desktop app, power users can edit)

Free tier power source options:
1. **Proxy with your API key** (Cloudflare Worker, rate-limited by install ID) - simpler
2. **Local model** (llama.cpp/candle, Phi-3-mini or Qwen2.5-3B) - no cost but +500MB

## Provider Config
```json
{
  "openai": { "base_url": "https://api.openai.com/v1", "default_model": "gpt-4o-mini" },
  "groq": { "base_url": "https://api.groq.com/openai/v1", "default_model": "llama-3.3-70b-versatile" },
  "anthropic": { "base_url": "https://api.anthropic.com/v1", "default_model": "claude-sonnet-4-20250514" },
  "openrouter": { "base_url": "https://openrouter.ai/api/v1", "default_model": "auto" }
}
```

OpenRouter is the simplest "one key for everything" option for users.

## Dependencies
```toml
reqwest = { version = "0.12", features = ["json", "stream"] }
reqwest-eventsource = "0.6"
keyring = "3"
async-trait = "0.1"
async-stream = "0.3"
futures = "0.3"
chrono = { version = "0.4", features = ["serde"] }
```
