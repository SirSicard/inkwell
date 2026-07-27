use async_trait::async_trait;
use serde_json::json;

/// Uniform result from LLM polish
pub struct PolishResult {
    pub text: String,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, system: &str, user: &str) -> Result<PolishResult, String>;
}

// ---------------------------------------------------------------------------
// OpenAI-compatible (OpenAI, Groq, Cerebras, OpenRouter, all on /v1/chat/completions)
// ---------------------------------------------------------------------------

pub struct OpenAICompatible {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
}

#[async_trait]
impl LlmProvider for OpenAICompatible {
    async fn complete(&self, system: &str, user: &str) -> Result<PolishResult, String> {
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&json!({
                "model": self.model,
                "messages": [
                    { "role": "system", "content": system },
                    { "role": "user",   "content": user }
                ],
                "max_tokens": 1024,
                "temperature": 0.3
            }))
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("API error {}: {}", status, body));
        }

        let body: serde_json::Value = resp.json().await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        let text = body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();

        Ok(PolishResult { text })
    }
}

// ---------------------------------------------------------------------------
// Anthropic (different auth header + different SSE format)
// ---------------------------------------------------------------------------

pub struct AnthropicProvider {
    pub model: String,
    pub api_key: String,
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn complete(&self, system: &str, user: &str) -> Result<PolishResult, String> {
        let client = reqwest::Client::new();
        let resp = client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&json!({
                "model": self.model,
                "system": system,
                "messages": [{ "role": "user", "content": user }],
                "max_tokens": 1024
            }))
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Anthropic API error {}: {}", status, body));
        }

        let body: serde_json::Value = resp.json().await
            .map_err(|e| format!("JSON parse error: {}", e))?;

        let text = body["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();

        Ok(PolishResult { text })
    }
}

// ---------------------------------------------------------------------------
// Factory: build provider from stored config
// ---------------------------------------------------------------------------

pub struct ProviderConfig {
    pub provider: String,  // "openai" | "groq" | "anthropic" | "openrouter" | "custom"
    pub api_key: String,
    pub custom_url: Option<String>,
    pub model: Option<String>,
}

/// Providers a BYOK key can be stored under, in preference order.
pub const PROVIDERS: &[&str] = &["openai", "groq", "anthropic", "openrouter", "custom"];

/// Look up an API key from the OS keyring. Keys are keyring-only, never on disk.
///
/// On macOS this is not a cheap read: the system asks the user to authorize
/// access to the item unless the calling binary is already in its ACL, so every
/// call is a potential password dialog. Call it when the key is about to be
/// used, never to find out whether one exists.
pub fn api_key_for(provider: &str) -> Option<String> {
    keyring::Entry::new("inkwell", provider)
        .ok()
        .and_then(|e| e.get_password().ok())
        .filter(|k| !k.is_empty())
}

/// Ask the keyring which providers have a key, by reading each one.
///
/// Deliberately named a probe rather than a getter: it costs one authorization
/// prompt per stored key on macOS. It exists only to reconcile an install that
/// predates `Settings::configured_providers`, and runs once.
pub fn probe_configured_providers() -> Vec<String> {
    PROVIDERS
        .iter()
        .filter(|p| api_key_for(p).is_some())
        .map(|p| p.to_string())
        .collect()
}

pub fn build_provider(cfg: ProviderConfig) -> Box<dyn LlmProvider> {
    match cfg.provider.as_str() {
        "anthropic" => Box::new(AnthropicProvider {
            model: cfg.model.unwrap_or_else(|| "claude-haiku-4-20250514".to_string()),
            api_key: cfg.api_key,
        }),
        "groq" => Box::new(OpenAICompatible {
            base_url: "https://api.groq.com/openai/v1".to_string(),
            model: cfg.model.unwrap_or_else(|| "llama-3.3-70b-versatile".to_string()),
            api_key: cfg.api_key,
        }),
        "openrouter" => Box::new(OpenAICompatible {
            base_url: "https://openrouter.ai/api/v1".to_string(),
            model: cfg.model.unwrap_or_else(|| "openai/gpt-4o-mini".to_string()),
            api_key: cfg.api_key,
        }),
        "custom" => Box::new(OpenAICompatible {
            base_url: cfg.custom_url.unwrap_or_else(|| "http://localhost:11434/v1".to_string()),
            model: cfg.model.unwrap_or_else(|| "llama3".to_string()),
            api_key: cfg.api_key,
        }),
        _ => Box::new(OpenAICompatible {  // default: openai
            base_url: "https://api.openai.com/v1".to_string(),
            model: cfg.model.unwrap_or_else(|| "gpt-4o-mini".to_string()),
            api_key: cfg.api_key,
        }),
    }
}

// ---------------------------------------------------------------------------
// Default polish prompt
// ---------------------------------------------------------------------------

pub const DEFAULT_POLISH_PROMPT: &str =
    "Clean up this speech-to-text transcription. The input comes from a dictation app and may contain:\
     \n- Filler words (um, uh, like, you know)\
     \n- False starts and repeated words\
     \n- Missing or wrong punctuation\
     \n- Misheard words or names\
     \n\
     \nRules:\
     \n- Fix grammar, punctuation, and capitalization\
     \n- Remove filler words and false starts\
     \n- Keep the speaker's original meaning, tone, and word choices\
     \n- Do NOT add, remove, or rephrase content\
     \n- Do NOT add greetings, sign-offs, or commentary\
     \n- Do NOT split into paragraphs (input is short dictation, not long-form)\
     \n- Return ONLY the cleaned text, nothing else";
