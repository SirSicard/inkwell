# Inkwell Voice Agent Mode: Integration Research
*2026-03-30 | Nix*

## Concept
Second hotkey (e.g. Ctrl+Shift+Space) for "agent mode". Same dictation pipeline (record, transcribe), but instead of pasting text, Inkwell sends the transcribed text to OpenClaw's gateway API. OpenClaw executes it, returns a response. Overlay shows the response.

## OpenClaw Gateway API Options

### Option A: /v1/chat/completions (OpenAI-compatible)
- **Endpoint:** `POST http://127.0.0.1:41738/v1/chat/completions`
- **Auth:** `Authorization: Bearer <OPENCLAW_GATEWAY_TOKEN>`
- **Agent selection:** `model: "openclaw:main"` or header `x-openclaw-agent-id: main`
- **Session persistence:** Include `"user": "inkwell-agent"` to derive a stable session key (conversations persist across calls)
- **Streaming:** `"stream": true` for SSE (would allow showing response as it generates)
- **Status:** Currently DISABLED. Needs `gateway.http.endpoints.chatCompletions.enabled: true` in config
- **Error seen:** `"missing scope: operator.write"` (needs enabling)
- **Pros:** Standard API, streaming support, session persistence via user field
- **Cons:** Needs config change to enable. Returns `"missing scope"` error currently.

### Option B: /tools/invoke (sessions_send)
- **Endpoint:** `POST http://127.0.0.1:41738/tools/invoke`
- **Auth:** `Authorization: Bearer <OPENCLAW_GATEWAY_TOKEN>`
- **Body:** `{"tool": "sessions_send", "args": {"message": "...", "sessionKey": "agent:main:main"}}`
- **Status:** WORKING (tested and confirmed)
- **Pros:** Already enabled, no config change needed
- **Cons:** `sessions_send` is on the HTTP hard deny list by default. Would need `gateway.tools.allow: ["sessions_send"]` in config. Also, fire-and-forget (no response returned inline).

### Option C: /tools/invoke (web_search, exec, etc.)
- Direct tool invocation, not a full agent turn
- Too limited for "do stuff for me" use case

### RECOMMENDED: Option A (/v1/chat/completions)
- One config change to enable
- Full agent turn with tools, memory, everything
- Streaming response for live overlay display
- Session persistence via `user: "inkwell-voice"` field
- Standard OpenAI-compatible interface (Inkwell already has reqwest + OpenAI client code in llm.rs)

## Config Change Needed

In `~/.openclaw/openclaw.json`, add:
```json
{
  "gateway": {
    "http": {
      "endpoints": {
        "chatCompletions": { "enabled": true }
      }
    }
  }
}
```

## What Needs to Change in Inkwell

### 1. Settings (settings.rs)
Add fields:
- `agent_hotkey: String` (default: "ctrl+shift+space")
- `agent_mode_enabled: bool` (default: false)
- `openclaw_url: String` (default: "http://127.0.0.1:41738")
- `openclaw_token: String` (stored in keyring like BYOK keys, not plaintext)
- `openclaw_agent_id: String` (default: "main")

### 2. Pipeline (pipeline.rs)
- Register a SECOND global shortcut for the agent hotkey
- Same recording pipeline: record > resample > VAD > transcribe
- Instead of style/dict/snippet/polish/paste, send to OpenClaw:
  ```
  POST {openclaw_url}/v1/chat/completions
  Authorization: Bearer {token}
  x-openclaw-agent-id: {agent_id}
  
  {
    "model": "openclaw",
    "user": "inkwell-voice",
    "stream": true,
    "messages": [{"role": "user", "content": "{transcribed_text}"}]
  }
  ```
- Parse streaming SSE response
- Emit response text to overlay via "agent-response" event

### 3. Overlay (overlay.html / overlay.rs)
- When agent mode is active, show the response in the overlay
- Glass pill with agent response text (similar to streaming text concept, but for the response)
- Auto-dismiss after 5-10 seconds, or click to dismiss
- Maybe a different accent color to distinguish from dictation mode

### 4. Frontend (src/tabs/GeneralTab.tsx or new AgentTab.tsx)
- Agent mode toggle
- Agent hotkey configuration
- OpenClaw connection settings: URL, token (stored securely), agent ID
- Connection test button
- Maybe a "recent agent interactions" log

### 5. Engine (NO CHANGES)
- Same transcription engine, same models
- Agent mode just changes what happens AFTER transcription

## Implementation Estimate
- **Rust changes:** ~150-200 lines (settings + second hotkey handler + HTTP client + SSE parser)
- **Frontend changes:** ~100-150 lines (settings UI for agent mode)
- **Overlay changes:** ~50 lines (agent response display)
- **Dependencies:** None new (reqwest already included)
- **Config:** One line in openclaw.json

## Flow Diagram
```
User holds Ctrl+Shift+Space
  → Audio captured (same pipeline)
  → Hotkey released
  → Resample → VAD → Transcribe
  → POST to OpenClaw /v1/chat/completions (streaming)
  → SSE response streams into overlay
  → Overlay shows: "Checking your calendar... You have a meeting at 3pm with Dawson."
  → Overlay auto-dismisses after 8 seconds
```

## Security Considerations
- Gateway token stored in OS keyring (same as BYOK API keys)
- Loopback only (127.0.0.1), no remote exposure
- Agent inherits all OpenClaw permissions/sandbox/tool policy
- User already has OpenClaw running, this just adds a voice interface to it

## Edge Cases
- OpenClaw not running: show error in overlay "OpenClaw not available"
- Long agent response: truncate in overlay, full response logged in agent history
- Agent still processing when user starts new recording: queue or ignore
- Token expired/wrong: show auth error in overlay
