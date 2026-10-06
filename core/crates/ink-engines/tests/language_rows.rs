//! Windows' language model row against the real file, locally: its size and hash are the file's,
//! and its answers hold no reasoning. Every test here is `#[ignore]`: CI has no model.
//!
//! `$INK_LLM_MODELS` names a directory holding the row's GGUF file, at any depth, under the name
//! the row gives it (`Qwen3-4B-Instruct-2507-Q4_K_M.gguf`).

use std::io::Read;
use std::path::{Path, PathBuf};

use ink_engines::{EngineRow, qwen3_4b_instruct_2507_q4km};
use sha2::{Digest, Sha256};

fn models_dir() -> PathBuf {
    std::env::var_os("INK_LLM_MODELS")
        .map(PathBuf::from)
        .expect("set INK_LLM_MODELS to the directory holding the language models' GGUF files")
}

/// The file named `name` at any depth under `dir`.
fn find(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        let kind = entry.file_type().ok()?;
        if kind.is_dir() {
            if let Some(found) = find(&path, name) {
                return Some(found);
            }
        } else if kind.is_file() && entry.file_name() == name {
            return Some(path);
        }
    }
    None
}

fn file_of(row: &EngineRow) -> PathBuf {
    let dir = models_dir();
    let name = &row.files[0].name;
    find(&dir, name).unwrap_or_else(|| panic!("no {name} under {}", dir.display()))
}

fn size_and_sha256(path: &Path) -> (u64, String) {
    let mut file = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 8 << 20];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    let sha = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    (size, sha)
}

#[test]
#[ignore = "hashes the language models' files under $INK_LLM_MODELS; run locally"]
fn the_language_row_matches_the_local_model_file() {
    let row = qwen3_4b_instruct_2507_q4km();
    let path = file_of(&row);
    let (size, sha) = size_and_sha256(&path);
    let f = &row.files[0];
    assert_eq!(
        (size, sha.as_str()),
        (f.size, f.sha256.as_str()),
        "{}",
        f.name
    );
    println!("{}: {size} bytes, sha256 {sha}: matches", f.name);
}

#[cfg(feature = "engine-llama")]
#[test]
#[ignore = "runs the language model from $INK_LLM_MODELS; run locally"]
fn the_language_models_answers_hold_no_reasoning() {
    use ink_core::{CancelToken, Llm, LlmRequest};
    use ink_engines::RowKind;
    use ink_engines::llama::LlamaLlm;

    let row = qwen3_4b_instruct_2507_q4km();
    let RowKind::Language(language) = &row.kind else {
        panic!("a language row")
    };
    let llm = LlamaLlm::load_with(&file_of(&row), &language.name, language.chat).unwrap();
    let request = LlmRequest {
        system: "Rewrite the user's text with correct punctuation. Reply with the text only."
            .into(),
        user: "so i think we should meet on tuesday at ten".into(),
        max_tokens: 96,
        temperature: 0.0,
        json_schema: None,
    };
    let answer = llm.complete(&request, &CancelToken::new()).unwrap();
    assert!(!answer.text.trim().is_empty());
    assert!(!answer.text.contains("<think>") && !answer.text.contains("</think>"));
}
