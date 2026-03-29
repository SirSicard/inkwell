# Chainable AI Transforms - Research

## Architecture
Sequential pipeline: each step takes text in, produces text out. Output of step N becomes input of step N+1. Streaming is per-step (can't stream across steps since step 2 needs step 1's full output).

```
[STT] → [Step 1: Fix Grammar] → [Step 2: Translate] → [Step 3: Summarize] → [Paste]
```

## Data Model
- **TransformStep**: id, type (fix_grammar/translate/summarize/expand/rewrite_tone/extract_actions/custom), prompt_template, model override, temperature, max_tokens, enabled, stop_on_error
- **TransformChain**: id, name, ordered steps array, global variables (language, tone), optional hotkey, isDefault flag
- Stored as JSON files in `$APPDATA/chains/` (one file per chain)

## Pipeline Engine (Rust)
- Use Tauri v2 `Channel` type for streaming tokens to frontend (faster than events)
- `execute_chain` command: iterates steps, renders prompt template, calls LLM, forwards tokens
- `catch_unwind` per step, skip or halt on error based on `stop_on_error` flag
- LLM abstraction trait: `stream_completion(prompt, model, temp, max_tokens) -> Stream<Token>`

## Prompt Template System
- Handlebars syntax: `{{text}}`, `{{original}}`, `{{language}}`, `{{tone}}`
- Use `handlebars` Rust crate
- Built-in variables: text (current), original (raw transcription), language, tone, style, step_number

## Built-in Transform Templates
- Fix Grammar: correct errors, preserve meaning
- Translate: to {{language}}, preserve formatting
- Summarize: 2-3 sentences, key points
- Expand: more detailed version
- Rewrite Tone: adjust to {{tone}} (professional, casual, friendly)
- Extract Actions: bullet list of action items

## LLM Provider Abstraction
- Trait-based: OpenAI, Anthropic, Ollama providers
- `reqwest` with streaming response for SSE APIs
- Each provider implements `stream_completion`

## UX
- Chain builder: drag-and-drop reordering of steps
- Live preview: shows output of each step as it streams
- Step cards with enable/disable toggle
- "Test chain" button with sample text
- Import/export chains as JSON
- Assign chains to hotkeys or make default

## Key Insight
Same approach as LangChain LCEL RunnableSequence but native Rust. No Python overhead. Each step is a uniform `async fn invoke(input) -> Result<output>`.
