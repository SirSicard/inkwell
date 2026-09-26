//! I5 for this crate: prompts, transcripts, keys and answers never reach a log or an error.
//!
//! The crate logs nothing at all, which a source scan can hold it to. Errors are checked by
//! behaviour: a canary in every input, and no canary in any error.

mod support;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use ink_core::{CancelToken, Llm, LlmRequest};
use ink_llm::tasks::commitments::parse_judgement;
use ink_llm::tasks::dedup::parse_verdict;
use ink_llm::tasks::summary::parse_summary;
use ink_llm::{ByokConfig, ByokLlm, LocalOnly, Provider, TransportError};
use support::{CountingTransport, MemKeys};

/// Every `.rs` file under `dir`.
fn sources(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push((
                path.display().to_string(),
                fs::read_to_string(&path).unwrap(),
            ));
        }
    }
}

#[test]
fn the_crate_has_no_logging_or_printing() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&src, &mut files);
    assert!(files.len() > 5, "found the sources");
    // Built from pieces so this file does not match itself if it is ever scanned.
    let banned = [
        ["print", "ln!("].concat(),
        ["eprint", "ln!("].concat(),
        ["print", "!("].concat(),
        ["eprint", "!("].concat(),
        ["dbg", "!("].concat(),
        ["log", "::"].concat(),
        ["tracing", "::"].concat(),
        ["debug", "!("].concat(),
        ["info", "!("].concat(),
        ["warn", "!("].concat(),
        ["error", "!("].concat(),
        ["trace", "!("].concat(),
    ];
    for (path, text) in &files {
        for pattern in &banned {
            assert!(
                !text.contains(pattern.as_str()),
                "{path} contains {pattern}"
            );
        }
    }
}

const CANARY: &str = "zebra-canary-7731";

fn canary_request() -> LlmRequest {
    LlmRequest {
        system: format!("system {CANARY}"),
        user: format!("user {CANARY}"),
        max_tokens: 16,
        temperature: 0.0,
        json_schema: None,
    }
}

#[test]
fn no_error_carries_the_prompt_the_key_or_the_answer() {
    let echo = format!(r#"{{"error": {{"message": "{CANARY}"}}}}"#);
    let answers: Vec<CountingTransport> = vec![
        CountingTransport::status(400, &echo),
        CountingTransport::status(500, &echo),
        CountingTransport::ok(&format!("not json {CANARY}")),
        CountingTransport::ok(&format!(
            r#"{{"choices": [{{"message": {{"content": 7}}}}], "x": "{CANARY}"}}"#
        )),
        CountingTransport::failing(TransportError::Io),
    ];
    for transport in answers {
        let transport = Arc::new(transport);
        let keys = Arc::new(MemKeys::with("openai", &format!("sk-{CANARY}")));
        let llm = ByokLlm::new(
            ByokConfig::new(Provider::OpenAi),
            keys,
            transport.clone(),
            LocalOnly::new(false),
        )
        .unwrap();
        let err = llm
            .complete(&canary_request(), &CancelToken::new())
            .unwrap_err();
        let shown = format!("{err} {err:?}");
        assert!(!shown.contains(CANARY), "{shown}");
    }
}

#[test]
fn shape_errors_name_fields_not_values() {
    let answers = [
        format!(
            r#"{{"class": "{CANARY}", "confidence": 1, "task": null, "due": null, "quote": "q"}}"#
        ),
        format!(
            r#"{{"class": "commitment", "confidence": "{CANARY}", "task": null, "due": null, "quote": "q"}}"#
        ),
        format!(r#"{{"headline": 1, "body": "{CANARY}"}}"#),
        format!(
            r#"{{"headline": "h", "body": "b", "decisions": [], "actions": [{{"text": 5, "owner": "{CANARY}"}}]}}"#
        ),
        format!(r#"{{"same": "{CANARY}", "keep": "A"}}"#),
        format!(r#"{{"same": true, "keep": "{CANARY}"}}"#),
        format!("{CANARY} with no JSON"),
    ];
    for answer in &answers {
        let errors = [
            parse_judgement(answer).err(),
            parse_summary(answer, true).err(),
            parse_verdict(answer).err(),
        ];
        for err in errors.into_iter().flatten() {
            let shown = format!("{err} {err:?}");
            assert!(!shown.contains(CANARY), "{shown}");
        }
    }
}
