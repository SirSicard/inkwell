//! A local GGUF chat model behind [`Llm`], for polish and summaries.
//!
//! Any GGUF chat model whose embedded chat template llama.cpp knows (ChatML, Llama 3, Gemma, Phi,
//! Mistral and the other built-in formats) works; a model without one is refused at load, not on
//! its first request. Every request gets a fresh context sized for its prompt and answer budget.
//!
//! - **Sampling.** Temperature 0 (or less, or not a number) is greedy. Above 0, llama.cpp's usual
//!   chain: top-k 40, top-p 0.95, min-p 0.05, the temperature, then a draw with a fixed seed, so
//!   the same request gives the same answer.
//! - **Structured answers.** With a [`LlmRequest::json_schema`], the answer is constrained to one
//!   JSON object by a grammar ([`JSON_OBJECT_GRAMMAR`]): the same guarantee the hosted providers'
//!   JSON mode gives. The schema itself is not enforced; the tasks describe it in their prompts
//!   and validate the answer, as they do for every provider.
//! - **Errors.** [`LlmError`] has no variant for a local engine that fails, so failures of this
//!   model are reported as [`LlmError::Network`] ("the request never got an answer"), with a
//!   message that starts `local model:`.

use std::num::NonZeroU32;
use std::path::Path;

use ink_core::{
    CancelToken, Endpoint, EngineError, Llm, LlmError, LlmInfo, LlmRequest, LlmResponse,
};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaChatTemplate, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;

use super::{GenerateError, backend, file_name, generate, require_file};

/// A grammar (llama.cpp's GBNF) for exactly one JSON object, with bounded whitespace so a model
/// cannot spend its budget on blank lines. Written for this crate from the JSON specification
/// (RFC 8259).
pub const JSON_OBJECT_GRAMMAR: &str = r#"
root   ::= object
value  ::= object | array | string | number | ("true" | "false" | "null") ws
object ::= "{" ws ( string ":" ws value ( "," ws string ":" ws value )* )? "}" ws
array  ::= "[" ws ( value ( "," ws value )* )? "]" ws
string ::= "\"" char* "\"" ws
char   ::= [^"\\\x7F\x00-\x1F] | "\\" ( ["\\/bfnrt] | "u" [0-9a-fA-F]{4} )
number ::= "-"? ( "0" | [1-9] [0-9]{0,15} ) ( "." [0-9]+ )? ( [eE] [-+]? [0-9]{1,4} )? ws
ws     ::= ( " " | "\n" [ \t]{0,20} )?
"#;

/// The seed for sampled (temperature above 0) answers.
const SEED: u32 = 0x5EED;

/// A GGUF chat model loaded in this process. Implements [`Llm`] with [`Endpoint::InProcess`], so
/// local-only mode allows it.
pub struct LlamaLlm {
    model: LlamaModel,
    template: LlamaChatTemplate,
    info: LlmInfo,
    backend: &'static LlamaBackend,
}

impl LlamaLlm {
    /// **Worker.** Loads the model at `path`, all layers on the GPU where there is one. `model_id`
    /// is what [`Llm::info`] reports. Refuses a model with no chat template, or one llama.cpp
    /// cannot apply.
    pub fn load(path: &Path, model_id: &str) -> Result<Self, EngineError> {
        require_file(path, model_id)?;
        let backend = backend()?;
        let failed = |what: String| EngineError::Failed(format!("{model_id}: {what}"));
        let model = LlamaModel::load_from_file(backend, path, &LlamaModelParams::default())
            .map_err(|e| failed(format!("llama.cpp could not load {}: {e}", file_name(path))))?;
        let template = model.chat_template(None).map_err(|e| {
            failed(format!(
                "{} has no usable chat template: {e}",
                file_name(path)
            ))
        })?;
        let probe = [message("system", "s")?, message("user", "u")?];
        model
            .apply_chat_template(&template, &probe, true)
            .map_err(|e| failed(format!("llama.cpp cannot apply its chat template: {e}")))?;
        Ok(Self {
            model,
            template,
            info: LlmInfo {
                provider: "local".into(),
                model: model_id.into(),
                endpoint: Endpoint::InProcess,
            },
            backend,
        })
    }
}

fn message(role: &str, content: &str) -> Result<LlamaChatMessage, EngineError> {
    LlamaChatMessage::new(role.into(), content.into())
        .map_err(|e| EngineError::Failed(format!("chat message: {e}")))
}

/// A failure of the local model. See the module docs for why it is `Network`.
fn local(what: impl std::fmt::Display) -> LlmError {
    LlmError::Network(format!("local model: {what}"))
}

impl Llm for LlamaLlm {
    fn info(&self) -> LlmInfo {
        self.info.clone()
    }

    fn complete(
        &self,
        request: &LlmRequest,
        cancel: &CancelToken,
    ) -> Result<LlmResponse, LlmError> {
        if cancel.is_cancelled() {
            return Err(LlmError::Cancelled);
        }
        if request.max_tokens == 0 {
            return Err(LlmError::BadResponse(
                "local model: the request allows no answer tokens".into(),
            ));
        }
        // User text with a NUL byte cannot cross into C; the error names the role, not the text.
        let chat_message = |role: &str, content: &str| {
            LlamaChatMessage::new(role.into(), content.into())
                .map_err(|_| local(format!("the {role} message contains a NUL character")))
        };
        let mut chat = Vec::with_capacity(2);
        if !request.system.trim().is_empty() {
            chat.push(chat_message("system", &request.system)?);
        }
        chat.push(chat_message("user", &request.user)?);
        let prompt = self
            .model
            .apply_chat_template(&self.template, &chat, true)
            .map_err(|e| local(format!("chat template: {e}")))?;
        // `Always` asks for the start token only where the model's vocabulary wants one.
        let tokens = self
            .model
            .str_to_token(&prompt, AddBos::Always)
            .map_err(|e| local(format!("tokenize: {e}")))?;
        // Sampling reads the last prompt token's logits, so there must be one.
        if tokens.is_empty() {
            return Err(local("the prompt is empty"));
        }

        let n_ctx = u32::try_from(tokens.len())
            .ok()
            .and_then(|n| n.checked_add(request.max_tokens))
            .filter(|&n| n <= self.model.n_ctx_train())
            .ok_or_else(|| {
                local(format!(
                    "a {}-token prompt plus {} answer tokens exceeds the model's context of {}",
                    tokens.len(),
                    request.max_tokens,
                    self.model.n_ctx_train()
                ))
            })?;
        let params = LlamaContextParams::default().with_n_ctx(NonZeroU32::new(n_ctx));
        let mut ctx = self
            .model
            .new_context(self.backend, params)
            .map_err(|e| local(format!("context: {e}")))?;

        // The prompt, in batches of the context's batch size; logits only for the last token.
        let n_batch = usize::try_from(ctx.n_batch()).unwrap_or(usize::MAX).max(1);
        let mut batch = LlamaBatch::new(n_batch.min(tokens.len()), 1);
        let last = tokens.len() - 1; // Not empty: checked above.
        for (c, chunk) in tokens.chunks(n_batch).enumerate() {
            if cancel.is_cancelled() {
                return Err(LlmError::Cancelled);
            }
            batch.clear();
            for (i, &token) in chunk.iter().enumerate() {
                let pos = c * n_batch + i;
                let pos_i32 = i32::try_from(pos).map_err(|_| local("prompt position overflow"))?;
                batch
                    .add(token, pos_i32, &[0], pos == last)
                    .map_err(|e| local(format!("batch: {e}")))?;
            }
            ctx.decode(&mut batch)
                .map_err(|e| local(format!("prompt decode: {e}")))?;
        }

        let mut sampler = self.sampler(request)?;
        let n_past = i32::try_from(tokens.len()).map_err(|_| local("prompt position overflow"))?;
        match generate(
            &self.model,
            &mut ctx,
            &mut sampler,
            n_past,
            request.max_tokens,
            cancel,
        ) {
            // Running out of budget is an answer cut short, as with any provider's length limit.
            Ok((text, _)) => Ok(LlmResponse { text }),
            Err(GenerateError::Cancelled) => Err(LlmError::Cancelled),
            Err(GenerateError::Failed(e)) => Err(local(e)),
        }
    }
}

impl LlamaLlm {
    fn sampler(&self, request: &LlmRequest) -> Result<LlamaSampler, LlmError> {
        let mut chain = Vec::with_capacity(6);
        if request.json_schema.is_some() {
            // First, so the others only ever see tokens the grammar allows.
            chain.push(
                LlamaSampler::grammar(&self.model, JSON_OBJECT_GRAMMAR, "root")
                    .map_err(|e| local(format!("JSON grammar: {e}")))?,
            );
        }
        let t = request.temperature;
        if t.is_finite() && t > 0.0 {
            chain.extend([
                LlamaSampler::top_k(40),
                LlamaSampler::top_p(0.95, 1),
                LlamaSampler::min_p(0.05, 1),
                LlamaSampler::temp(t),
                LlamaSampler::dist(SEED),
            ]);
        } else {
            chain.push(LlamaSampler::greedy());
        }
        Ok(LlamaSampler::chain_simple(chain))
    }
}
