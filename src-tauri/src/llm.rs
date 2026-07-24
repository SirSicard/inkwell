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
// OpenAI-compatible (OpenAI, Groq, Cerebras, OpenRouter — same /v1/chat/completions)
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

// ---------------------------------------------------------------------------
// Proxy (free tier) — calls the Inkwell Cloudflare Worker
// ---------------------------------------------------------------------------

/// The hosted proxy URL. Update after deploying the Worker.
pub const PROXY_BASE_URL: &str = "https://inkwell-polish.mattias-e67.workers.dev";

#[derive(Debug, serde::Deserialize)]
pub struct ProxyResponse {
    pub text: Option<String>,
    pub error: Option<String>,
    pub message: Option<String>,
    pub words_used: Option<u32>,
    pub limit: Option<u32>,
    pub remaining: Option<u32>,
    pub resets_on: Option<String>,
}

pub async fn call_proxy(
    install_id: &str,
    text: &str,
    system_prompt: &str,
) -> Result<ProxyResponse, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/polish", PROXY_BASE_URL))
        .header("Content-Type", "application/json")
        .header("X-Inkwell-Version", env!("CARGO_PKG_VERSION"))
        .json(&serde_json::json!({
            "install_id": install_id,
            "text": text,
            "system_prompt": system_prompt,
        }))
        .send()
        .await
        .map_err(|e| format!("Proxy request failed: {}", e))?;

    let status = resp.status();
    let body: ProxyResponse = resp.json().await
        .map_err(|e| format!("Proxy response parse error: {}", e))?;

    if status == 429 {
        return Err(body.message.unwrap_or_else(|| "Weekly limit reached".to_string()));
    }

    if !status.is_success() {
        return Err(body.error.unwrap_or_else(|| format!("Proxy error {}", status)));
    }

    Ok(body)
}
