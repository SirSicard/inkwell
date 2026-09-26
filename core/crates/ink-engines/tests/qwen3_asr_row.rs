//! The built-in Qwen3-ASR row: what it pins, and (locally, with the model present) that its sizes
//! and hashes are the files'.

mod bench;

use std::io::Read;

use ink_core::Job;
use ink_engines::{Os, Registry, Runtime};
use sha2::{Digest, Sha256};

const ID: &str = "qwen3-asr-1.7b-q8";
const REVISION: &str = "36a678687ba7d07a74ca70ccb0e36902e005fb80";

#[test]
fn the_qwen3_asr_row_is_pinned_licensed_and_mac_only() {
    let registry = Registry::builtin().expect("built-in rows validate");
    let row = registry.get(ID).expect("the Qwen3-ASR row is built in");
    assert_eq!(row.revision, REVISION);
    // The base model's licence (the GGUF conversion's repository has no tag).
    assert_eq!(row.licence, "Apache-2.0");
    assert_eq!(row.runtime, Runtime::LlamaCpp);
    // Windows waits for its own speed measurement.
    assert_eq!(row.oses, vec![Os::MacOs]);
    // AMI IHM for meetings, FLEURS English as published for dictation.
    assert_eq!(row.wer(Job::MeetingFinal), Some(16.08));
    assert_eq!(row.wer(Job::DictationFinal), Some(4.59));
    assert_eq!(row.wer(Job::LivePartials), None);

    let names: Vec<&str> = row.files.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Qwen3-ASR-1.7B-Q8_0.gguf",
            "mmproj-Qwen3-ASR-1.7B-Q8_0.gguf"
        ]
    );
    for f in &row.files {
        assert_eq!(
            f.url,
            format!(
                "https://huggingface.co/ggml-org/Qwen3-ASR-1.7B-GGUF/resolve/{REVISION}/{}",
                f.name
            )
        );
    }
    assert_eq!(row.total_size(), 2_165_034_944 + 355_709_344);
}

#[test]
#[ignore = "hashes the local Qwen3-ASR files under $INK_BENCH_DIR; run locally"]
fn the_qwen3_asr_row_matches_the_local_model_files() {
    let dir = bench::bench_dir().join("models/qwen3-asr-1.7b-gguf");
    let registry = Registry::builtin().unwrap();
    let row = registry.get(ID).unwrap();
    for f in &row.files {
        let path = dir.join(&f.name);
        let mut file =
            std::fs::File::open(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
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
        let sha: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            (size, sha.as_str()),
            (f.size, f.sha256.as_str()),
            "{}",
            f.name
        );
    }
}
