//! The model registry: one table, every consumer.
//!
//! This used to be six hardcoded `match` statements (install check, switch,
//! download file list, download base URL, directory name, active check) plus
//! two more in `setup.rs` and a hand-maintained `MODEL_CATALOG` in the
//! frontend. Adding a model meant editing eight places, and forgetting one was
//! silent: `moonshine-tiny` shipped in the UI catalogue with no download arm,
//! so its button returned `Unknown model`, and it was simultaneously the last
//! entry in the startup fallback chain, a fallback that could never load.
//!
//! Everything now reads from `MODELS`. The frontend gets the same data over
//! `list_models`, so the catalogue cannot drift from what the backend can
//! actually fetch and load.

use crate::engine::SpeechEngine;
use serde::Serialize;
use std::path::Path;

/// Which `SpeechEngine` constructor loads this model.
#[derive(Debug, Clone, Copy)]
pub enum EngineKind {
    /// Parakeet directory suffix: "v3", "v2", "v3-fp32", "v2-fp16"...
    /// Carries the variant rather than hardcoding two, so a new precision or
    /// language build is a table row instead of an engine change.
    Parakeet(&'static str),
    /// Moonshine variant name, e.g. "base".
    Moonshine(&'static str),
    /// Whisper variant name, e.g. "tiny", "large-v3".
    Whisper(&'static str),
    SenseVoice,
}

pub struct ModelSpec {
    /// Stable id used in settings, commands and the frontend.
    pub id: &'static str,
    /// Human name. Also what `AppState.model_name` holds once loaded, so the
    /// active-model check compares against this rather than re-deriving it.
    pub display: &'static str,
    /// Directory under the models dir. Often but not always the id.
    pub dir: &'static str,
    pub company: &'static str,
    pub description: &'static str,
    /// Human size for the UI, e.g. "670 MB".
    pub size: &'static str,
    pub languages: &'static str,
    pub hf_base: &'static str,
    /// Files to fetch, with approximate byte sizes for download progress.
    pub files: &'static [(&'static str, u64)],
    /// Any one of these existing means the model is installed. Several names
    /// are accepted because int8 and float builds ship different filenames.
    pub encoder_files: &'static [&'static str],
    pub kind: EngineKind,
}

impl ModelSpec {
    pub fn dir_in(&self, models_dir: &Path) -> std::path::PathBuf {
        models_dir.join(self.dir)
    }

    /// Is the model present on disk? Presence only; this does not verify that
    /// a partially downloaded file is complete.
    pub fn is_installed(&self, models_dir: &Path) -> bool {
        let dir = self.dir_in(models_dir);
        self.encoder_files.iter().any(|f| dir.join(f).exists())
    }

    pub fn load(&self, models_dir: &Path) -> Result<SpeechEngine, String> {
        match self.kind {
            EngineKind::Parakeet(v) => SpeechEngine::parakeet_variant(models_dir, v),
            EngineKind::Moonshine(v) => SpeechEngine::moonshine(models_dir, v),
            EngineKind::Whisper(v) => SpeechEngine::whisper(models_dir, v),
            EngineKind::SenseVoice => SpeechEngine::sense_voice(models_dir),
        }
    }

    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|(_, size)| size).sum()
    }
}

/// What the frontend renders. Kept separate from `ModelSpec` so download URLs
/// and file lists stay in the backend.
#[derive(Serialize)]
pub struct ModelInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub company: &'static str,
    pub description: &'static str,
    pub size: &'static str,
    pub languages: &'static str,
    pub installed: bool,
}

const ENCODER_FILES: &[&str] = &[
    "encoder.int8.onnx",
    "encoder.onnx",
    "encoder.fp16.onnx",
    "encode.int8.onnx",
    "encode.onnx",
];

/// Order is the order the UI shows them in: recommended default first.
pub const MODELS: &[ModelSpec] = &[
    ModelSpec {
        id: "parakeet",
        display: "Parakeet V3",
        dir: "parakeet-v3",
        company: "NVIDIA",
        description: "Fast and accurate. Detects which of 25 European languages you are speaking, with no setting to change.",
        size: "670 MB",
        languages: "25 languages",
        hf_base: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main",
        files: &[
            ("encoder.int8.onnx", 683_000_000),
            ("decoder.int8.onnx", 12_000_000),
            ("joiner.int8.onnx", 7_000_000),
            ("tokens.txt", 96_000),
        ],
        encoder_files: ENCODER_FILES,
        kind: EngineKind::Parakeet("v3"),
    },
    ModelSpec {
        id: "parakeet-v2",
        display: "Parakeet V2",
        dir: "parakeet-v2",
        company: "NVIDIA",
        description: "English-specialized. Same architecture as V3 but tuned for English.",
        size: "670 MB",
        languages: "English",
        hf_base: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/resolve/main",
        files: &[
            ("encoder.int8.onnx", 683_000_000),
            ("decoder.int8.onnx", 12_000_000),
            ("joiner.int8.onnx", 7_000_000),
            ("tokens.txt", 96_000),
        ],
        encoder_files: ENCODER_FILES,
        kind: EngineKind::Parakeet("v2"),
    },
    ModelSpec {
        id: "parakeet-v2-fp16",
        display: "Parakeet V2 (fp16)",
        dir: "parakeet-v2-fp16",
        company: "NVIDIA",
        description: "English only, half precision instead of int8. Bigger download for whatever accuracy quantisation was costing. Experimental: compare it against Parakeet V3 on your own voice before keeping it.",
        size: "1.3 GB",
        languages: "English",
        hf_base: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-fp16/resolve/main",
        files: &[
            ("encoder.fp16.onnx", 1_239_245_548),
            ("decoder.fp16.onnx", 14_446_596),
            ("joiner.fp16.onnx", 3_456_459),
            ("tokens.txt", 9_384),
        ],
        encoder_files: ENCODER_FILES,
        kind: EngineKind::Parakeet("v2-fp16"),
    },
    ModelSpec {
        id: "parakeet-v3-fp32",
        display: "Parakeet V3 (full precision)",
        dir: "parakeet-v3-fp32",
        company: "NVIDIA",
        description: "The same 25-language model as the default, unquantised. 2.5 GB, and the encoder's weights arrive as a separate file. Experimental: this exists to measure what int8 costs on your voice, not because it is the better default.",
        size: "2.5 GB",
        languages: "25 languages",
        hf_base: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3/resolve/main",
        files: &[
            ("encoder.onnx", 41_766_257),
            // ONNX external data: encoder.onnx references this by name and
            // onnxruntime resolves it beside the model, so the two must land in
            // the same directory or loading fails at runtime, not at download.
            ("encoder.weights", 2_435_420_160),
            ("decoder.onnx", 47_233_743),
            ("joiner.onnx", 25_286_330),
            ("tokens.txt", 93_939),
        ],
        encoder_files: ENCODER_FILES,
        kind: EngineKind::Parakeet("v3-fp32"),
    },
    ModelSpec {
        id: "whisper-turbo",
        display: "Whisper Turbo",
        dir: "whisper-turbo",
        company: "OpenAI",
        description: "Balanced accuracy and speed.",
        size: "800 MB",
        languages: "99 languages",
        hf_base: "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-turbo/resolve/main",
        files: &[
            ("turbo-encoder.int8.onnx", 397_000_000),
            ("turbo-decoder.int8.onnx", 409_000_000),
            ("turbo-tokens.txt", 800_000),
        ],
        encoder_files: &["turbo-encoder.int8.onnx", "turbo-encoder.onnx"],
        kind: EngineKind::Whisper("turbo"),
    },
    ModelSpec {
        id: "whisper-large-v3",
        display: "Whisper large-v3",
        dir: "whisper-large-v3",
        company: "OpenAI",
        description: "Best accuracy, but slow.",
        size: "1.5 GB",
        languages: "99 languages",
        hf_base: "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-large-v3/resolve/main",
        files: &[
            ("large-v3-encoder.int8.onnx", 397_000_000),
            ("large-v3-decoder.int8.onnx", 1_100_000_000),
            ("large-v3-tokens.txt", 800_000),
        ],
        encoder_files: &["large-v3-encoder.int8.onnx", "large-v3-encoder.onnx"],
        kind: EngineKind::Whisper("large-v3"),
    },
    ModelSpec {
        id: "whisper-medium",
        display: "Whisper medium",
        dir: "whisper-medium",
        company: "OpenAI",
        description: "Good accuracy, medium speed.",
        size: "1.0 GB",
        languages: "99 languages",
        hf_base: "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-medium/resolve/main",
        files: &[
            ("medium-encoder.int8.onnx", 193_000_000),
            ("medium-decoder.int8.onnx", 823_000_000),
            ("medium-tokens.txt", 800_000),
        ],
        encoder_files: &["medium-encoder.int8.onnx", "medium-encoder.onnx"],
        kind: EngineKind::Whisper("medium"),
    },
    ModelSpec {
        id: "whisper-small",
        display: "Whisper small",
        dir: "whisper-small",
        company: "OpenAI",
        description: "Fast and fairly accurate.",
        size: "375 MB",
        languages: "99 languages",
        hf_base: "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-small/resolve/main",
        files: &[
            ("small-encoder.int8.onnx", 72_000_000),
            ("small-decoder.int8.onnx", 305_000_000),
            ("small-tokens.txt", 800_000),
        ],
        encoder_files: &["small-encoder.int8.onnx", "small-encoder.onnx"],
        kind: EngineKind::Whisper("small"),
    },
    ModelSpec {
        id: "whisper-base",
        display: "Whisper base",
        dir: "whisper-base",
        company: "OpenAI",
        description: "Lightweight multilingual.",
        size: "135 MB",
        languages: "99 languages",
        hf_base: "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-base/resolve/main",
        files: &[
            ("base-encoder.int8.onnx", 20_000_000),
            ("base-decoder.int8.onnx", 114_000_000),
            ("base-tokens.txt", 800_000),
        ],
        encoder_files: &["base-encoder.int8.onnx", "base-encoder.onnx"],
        kind: EngineKind::Whisper("base"),
    },
    ModelSpec {
        id: "whisper-tiny",
        display: "Whisper tiny",
        dir: "whisper-tiny",
        company: "OpenAI",
        description: "Smallest Whisper model.",
        size: "98 MB",
        languages: "99 languages",
        hf_base: "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-tiny/resolve/main",
        files: &[
            ("tiny-encoder.int8.onnx", 12_000_000),
            ("tiny-decoder.int8.onnx", 86_000_000),
            ("tiny-tokens.txt", 800_000),
        ],
        encoder_files: &["tiny-encoder.int8.onnx", "tiny-encoder.onnx"],
        kind: EngineKind::Whisper("tiny"),
    },
    ModelSpec {
        id: "whisper-distil-medium-en",
        display: "Whisper distil-medium.en",
        dir: "whisper-distil-medium-en",
        company: "OpenAI",
        description: "English only. Distilled for speed.",
        size: "460 MB",
        languages: "English",
        hf_base: "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-distil-medium.en/resolve/main",
        files: &[
            ("distil-medium.en-encoder.int8.onnx", 193_000_000),
            ("distil-medium.en-decoder.int8.onnx", 270_000_000),
            ("distil-medium.en-tokens.txt", 800_000),
        ],
        encoder_files: &[
            "distil-medium.en-encoder.int8.onnx",
            "distil-medium.en-encoder.onnx",
        ],
        kind: EngineKind::Whisper("distil-medium.en"),
    },
    ModelSpec {
        id: "whisper-distil-small-en",
        display: "Whisper distil-small.en",
        dir: "whisper-distil-small-en",
        company: "OpenAI",
        description: "English only. Compact and fast.",
        size: "180 MB",
        languages: "English",
        hf_base: "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-distil-small.en/resolve/main",
        files: &[
            ("distil-small.en-encoder.int8.onnx", 72_000_000),
            ("distil-small.en-decoder.int8.onnx", 108_000_000),
            ("distil-small.en-tokens.txt", 800_000),
        ],
        encoder_files: &[
            "distil-small.en-encoder.int8.onnx",
            "distil-small.en-encoder.onnx",
        ],
        kind: EngineKind::Whisper("distil-small.en"),
    },
    ModelSpec {
        id: "moonshine-base",
        display: "Moonshine Base",
        dir: "moonshine-base",
        company: "Useful Sensors",
        description: "English only. Good accuracy, fast inference.",
        size: "288 MB",
        languages: "English",
        hf_base: "https://huggingface.co/csukuangfj/sherpa-onnx-moonshine-base-en-int8/resolve/main",
        files: &[
            ("preprocess.onnx", 14_100_000),
            ("encode.int8.onnx", 50_300_000),
            ("cached_decode.int8.onnx", 100_000_000),
            ("uncached_decode.int8.onnx", 122_000_000),
            ("tokens.txt", 437_000),
        ],
        encoder_files: ENCODER_FILES,
        kind: EngineKind::Moonshine("base"),
    },
    ModelSpec {
        id: "sense-voice",
        display: "SenseVoice",
        dir: "sense-voice",
        company: "Alibaba",
        description: "Very fast. Detects Chinese, English, Japanese, Korean and Cantonese automatically.",
        size: "160 MB",
        languages: "5 languages",
        hf_base: "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main",
        files: &[
            ("model.int8.onnx", 160_000_000),
            ("tokens.txt", 50_000),
        ],
        encoder_files: &["model.int8.onnx", "model.onnx"],
        kind: EngineKind::SenseVoice,
    },
];

/// The model loaded when the user has expressed no preference.
pub const DEFAULT_MODEL_ID: &str = "parakeet";

pub fn find(id: &str) -> Option<&'static ModelSpec> {
    MODELS.iter().find(|m| m.id == id)
}

/// Catalogue plus per-model installed state, for the UI.
pub fn catalog(models_dir: &Path) -> Vec<ModelInfo> {
    MODELS
        .iter()
        .map(|m| ModelInfo {
            id: m.id,
            name: m.display,
            company: m.company,
            description: m.description,
            size: m.size,
            languages: m.languages,
            installed: m.is_installed(models_dir),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<_> = MODELS.iter().map(|m| m.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate model id in MODELS");
    }

    #[test]
    fn dirs_are_unique() {
        // Two models sharing a directory would make install checks and removal
        // interfere with each other.
        let mut dirs: Vec<_> = MODELS.iter().map(|m| m.dir).collect();
        dirs.sort_unstable();
        let before = dirs.len();
        dirs.dedup();
        assert_eq!(before, dirs.len(), "duplicate model dir in MODELS");
    }

    #[test]
    fn the_default_model_exists() {
        assert!(find(DEFAULT_MODEL_ID).is_some());
    }

    #[test]
    fn every_model_is_downloadable() {
        // This is the invariant moonshine-tiny broke: it was listed for the UI
        // but had no files and no base URL, so Download returned Unknown model.
        for m in MODELS {
            assert!(!m.files.is_empty(), "{} has no files to download", m.id);
            assert!(m.hf_base.starts_with("https://"), "{} has no base URL", m.id);
            assert!(
                !m.encoder_files.is_empty(),
                "{} can never be detected as installed",
                m.id
            );
        }
    }

    #[test]
    fn install_check_matches_a_downloaded_file() {
        // If no encoder_files entry is among the downloaded files, the model
        // would install successfully and still report itself missing forever.
        for m in MODELS {
            let downloaded: Vec<&str> = m.files.iter().map(|(n, _)| *n).collect();
            assert!(
                m.encoder_files.iter().any(|e| downloaded.contains(e)),
                "{}: none of {:?} are downloaded ({:?})",
                m.id,
                m.encoder_files,
                downloaded
            );
        }
    }

    #[test]
    fn sizes_are_plausible() {
        for m in MODELS {
            assert!(m.total_bytes() > 1_000_000, "{} looks too small", m.id);
        }
    }
}
