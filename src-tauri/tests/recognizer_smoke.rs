//! End-to-end recognizer smoke test against a real installed model.
//!
//! Ignored by default: it needs Parakeet V3 on disk (~640 MB), which CI does not
//! have. Run it after installing a model to prove the whole audio path works:
//!
//!     say -o /tmp/inkwell_test.wav --data-format=LEI16@16000 \
//!         "the quick brown fox jumps over the lazy dog"
//!     cargo test --test recognizer_smoke -- --ignored --nocapture
//!
//! This covers the seam the unit tests cannot: sherpa-onnx model loading, the
//! decode path, and transcription of real speech.

use app_lib::{engine, filetranscribe};
use std::path::PathBuf;

fn models_dir() -> PathBuf {
    // Matches setup.rs: app_data_dir()/models
    let home = std::env::var("HOME").expect("HOME not set");
    PathBuf::from(home)
        .join("Library/Application Support/com.inkwell.app/models")
}

#[test]
#[ignore = "needs an installed model and a generated wav"]
fn transcribes_known_speech() {
    let wav = PathBuf::from("/tmp/inkwell_test.wav");
    assert!(wav.exists(), "generate the fixture first (see module docs)");

    let samples = filetranscribe::decode_to_pcm(&wav).expect("decode failed");
    assert!(!samples.is_empty(), "decoded to zero samples");
    println!("decoded {} samples", samples.len());

    let eng = engine::SpeechEngine::parakeet(&models_dir()).expect("model load failed");
    let text = eng.transcribe(&samples).expect("transcribe failed");
    println!("transcript: {text:?}");

    // Compare loosely: recognizers vary on casing and punctuation.
    let got = text.to_lowercase();
    for word in ["quick", "brown", "fox", "lazy", "dog"] {
        assert!(got.contains(word), "expected {word:?} in transcript, got {text:?}");
    }
}
