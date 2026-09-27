//! Language models from a shell engine (`INK_ENGINE_LLM`), as an [`Llm`]: Foundation Models on
//! the Mac, for dictation polish.
//!
//! The request goes out as JSON and the answer comes back as text or an error kind. An engine that
//! cannot run now (Apple Intelligence off, not supported on this Mac, its model not ready) answers
//! `unavailable`, which reaches the pipeline as an [`LlmError`]: polish then keeps the dictation
//! as written and says why. Nothing here ever stands in for an answer.
//!
//! Where the model runs is the shell's declaration (`"local"` in its info): it is what the
//! local-only guard (architecture rule 6) reads through [`Llm::info`].

use std::ffi::CString;
use std::time::Duration;

use ink_core::{CancelToken, Endpoint, Llm, LlmError, LlmInfo, LlmRequest, LlmResponse};
use serde_json::{Value, json};

use super::{Answer, Call, Expect, GaveUp, GenerateFn, Shell, ShellError};

/// How long a generation may take: this only stops an engine that never answers from holding a
/// job forever. Dictation polish cancels its call much sooner (`ink_pipeline::chain::POLISH_BUDGET`),
/// since a take waits on it.
pub const GENERATE_TIMEOUT: Duration = Duration::from_secs(120);

/// A language model's `info_json`, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Info {
    pub(crate) id: String,
    pub(crate) licence: String,
    pub(crate) model: String,
    pub(crate) local: bool,
}

/// `{"id","licence","model","local"}`, all required.
pub(crate) fn parse_info(json: &str) -> Option<Info> {
    let v: Value = serde_json::from_str(json).ok()?;
    let id = v.get("id")?.as_str()?.to_owned();
    if id.trim().is_empty() {
        return None;
    }
    Some(Info {
        id,
        licence: v.get("licence")?.as_str()?.to_owned(),
        model: v.get("model")?.as_str()?.to_owned(),
        local: v.get("local")?.as_bool()?,
    })
}

/// A registered language model. Its `release` runs when the last reference drops.
pub struct ExternalLlm {
    shell: Shell,
    generate: GenerateFn,
    info: Info,
}

impl ExternalLlm {
    pub(crate) fn new(shell: Shell, generate: GenerateFn, info: Info) -> Self {
        Self {
            shell,
            generate,
            info,
        }
    }

    /// The id it registered under.
    pub fn id(&self) -> &str {
        &self.info.id
    }

    /// The weights' licence.
    pub fn licence(&self) -> &str {
        &self.info.licence
    }

    /// Makes the drop skip `release`: for a table the core refused.
    pub fn disarm(&self) {
        self.shell.disarm();
    }
}

/// The request as the header gives it. It holds the user's words: never logged.
fn request_json(request: &LlmRequest) -> String {
    let mut v = json!({
        "system": request.system,
        "user": request.user,
        "max_tokens": request.max_tokens,
        "temperature": request.temperature,
    });
    if let Some(schema) = &request.json_schema {
        v["json_schema"] = Value::from(schema.as_str());
    }
    v.to_string()
}

impl Llm for ExternalLlm {
    fn info(&self) -> LlmInfo {
        LlmInfo {
            provider: "shell".into(),
            model: self.info.model.clone(),
            endpoint: if self.info.local {
                Endpoint::InProcess
            } else {
                // Named by the engine's id: its address, if it has one, is the shell's business.
                Endpoint::Remote(format!("shell engine {}", self.info.id))
            },
        }
    }

    fn complete(
        &self,
        request: &LlmRequest,
        cancel: &CancelToken,
    ) -> Result<LlmResponse, LlmError> {
        if cancel.is_cancelled() {
            return Err(LlmError::Cancelled);
        }
        let json = CString::new(request_json(request))
            .map_err(|_| LlmError::Engine("the request held a NUL byte".into()))?;
        let call = Call::open(Expect::Text);
        let id = call.id();
        // SAFETY: the shell's function with its own `ctx`, valid until `release`, which cannot
        // run while `self` is alive. The request is valid until the call returns.
        unsafe { (self.generate)(self.shell.ctx, id, json.as_ptr()) };
        let cancelled = || cancel.is_cancelled() || self.shell.shutting_down();
        match call.wait(cancelled, Some(GENERATE_TIMEOUT), || {
            self.shell.cancel_call(id)
        }) {
            Ok(Ok(Answer::Text(text))) => Ok(LlmResponse { text }),
            Ok(Ok(_)) => Err(ShellError::UNREADABLE.llm_error(&self.info.id)),
            Ok(Err(e)) => Err(e.llm_error(&self.info.id)),
            Err(GaveUp::Cancelled) => Err(LlmError::Cancelled),
            Err(GaveUp::TimedOut) => Err(LlmError::Engine(format!(
                "shell engine {} did not answer within {GENERATE_TIMEOUT:?}",
                self.info.id
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_needs_every_field() {
        let info =
            parse_info(r#"{"id":"fm","licence":"Apple","model":"system","local":true}"#).unwrap();
        assert_eq!(info.model, "system");
        assert!(info.local);
        for bad in [
            r#"{"id":"fm","licence":"Apple","model":"system"}"#,
            r#"{"id":"fm","licence":"Apple","local":true}"#,
            r#"{"id":"","licence":"Apple","model":"m","local":true}"#,
            r#"{"id":"fm","licence":"Apple","model":"m","local":"yes"}"#,
            r#"{"id":"fm","model":"m","local":true}"#,
        ] {
            assert!(parse_info(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_request_carries_every_field_and_the_schema_only_when_set() {
        let mut r = LlmRequest {
            system: "sys".into(),
            user: "synthetic words".into(),
            max_tokens: 64,
            temperature: 0.25,
            json_schema: None,
        };
        let v: Value = serde_json::from_str(&request_json(&r)).unwrap();
        assert_eq!(v["system"], "sys");
        assert_eq!(v["user"], "synthetic words");
        assert_eq!(v["max_tokens"], 64);
        assert_eq!(v["temperature"], 0.25);
        assert!(v.get("json_schema").is_none());
        r.json_schema = Some("{}".into());
        let v: Value = serde_json::from_str(&request_json(&r)).unwrap();
        assert_eq!(v["json_schema"], "{}");
    }
}
