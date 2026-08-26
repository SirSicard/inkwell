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
    Qwen3,
    /// A streaming transducer, loaded by `streaming.rs` rather than by
    /// `SpeechEngine`. It shares `ModelSpec` for its download metadata and
    /// nothing else; `load` refuses it on purpose.
    Streaming,
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
            EngineKind::Qwen3 => SpeechEngine::qwen3(models_dir),
            EngineKind::Streaming => Err(format!(
                "{} is a streaming model and cannot transcribe a finished recording",
                self.display
            )),
        }
    }

    /// Does this model read its bias phrases at construction rather than per
    /// utterance? If so the engine must be rebuilt when the dictionary changes,
    /// and `write_hotwords` must have run first.
    pub fn hotwords_at_load(&self) -> bool {
        matches!(self.kind, EngineKind::Qwen3)
    }

    /// Write bias phrases where a construction-time model looks for them.
    /// Writes an empty file rather than deleting, so "no hotwords" stays
    /// distinguishable from "never written" when debugging a misheard name.
    pub fn write_hotwords(&self, models_dir: &Path, hotwords: Option<&str>) -> Result<(), String> {
        if !self.hotwords_at_load() {
            return Ok(());
        }
        let path = self.dir_in(models_dir).join("hotwords.txt");
        std::fs::write(&path, hotwords.unwrap_or(""))
            .map_err(|e| format!("Could not write {}: {}", path.display(), e))
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
    // Curated, not comprehensive. This was thirteen models, which asked the user
    // to research speech recognition before they could dictate a sentence, and
    // most of them lost on every axis at once: the small Whisper builds were
    // less accurate AND slower than Parakeet, and the English-distilled ones
    // were beaten by Parakeet V2 at a similar size.
    //
    // What is left answers four different questions, measured on real recordings
    // (see the ab_models example): which language do I speak, how good does it
    // need to be, and how much disk and time can I spare.
    //
    // Dropped after measuring: Parakeet V2 fp16 (identical accuracy to V2 int8
    // for twice the download), Moonshine Base (SenseVoice is smaller and more
    // accurate), Whisper large-v3/medium/small/base/tiny and the two distil-en
    // builds (all dominated by something above), Parakeet V3 full precision
    // (external-data weights this build cannot load; it aborts the process).
    ModelSpec {
        id: "parakeet",
        display: "Parakeet V3",
        dir: "parakeet-v3",
        company: "NVIDIA",
        description: "Detects which of 25 European languages you are speaking, with no setting to change. The safe default, and the only choice here if you switch languages mid-sentence. If you only ever dictate in English, Parakeet V2 is measurably better.",
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
        description: "The most accurate option for English: 8.0% word error rate against 10.5% for the multilingual default, on the same recordings, at the same download size. Gets names and product words right where the others do not. English only.",
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
        id: "sense-voice",
        display: "SenseVoice",
        dir: "sense-voice",
        company: "Alibaba",
        description: "A quarter of the size and twice the speed of the others, and still close to the best: 9.3% word error rate. The right pick on a small disk, a slow connection, or an older machine, and the only small model here that also handles Chinese, Japanese, Korean and Cantonese.",
        size: "240 MB",
        languages: "5 languages",
        hf_base: "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main",
        files: &[
            ("model.int8.onnx", 160_000_000),
            ("tokens.txt", 50_000),
        ],
        encoder_files: &["model.int8.onnx", "model.onnx"],
        kind: EngineKind::SenseVoice,
    },
    ModelSpec {
        id: "qwen3-asr",
        display: "Qwen3 ASR",
        dir: "qwen3-asr",
        company: "Alibaba",
        description: "The most accurate option measured here: 5.6% word error rate against 8.0% for Parakeet V2, on the same recordings. It is the only model that covers both English and the Nordic languages, so it does not force a choice between them. Costs a bigger download and about twice the transcription time, which is still under a second for a short dictation.",
        size: "940 MB",
        languages: "30 languages",
        hf_base: "https://huggingface.co/csukuangfj/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25/resolve/main",
        files: &[
            ("conv_frontend.onnx", 43_000_000),
            ("encoder.int8.onnx", 182_000_000),
            ("decoder.int8.onnx", 756_000_000),
        ],
        encoder_files: &["encoder.int8.onnx", "encoder.onnx"],
        kind: EngineKind::Qwen3,
    },
    ModelSpec {
        id: "whisper-turbo",
        display: "Whisper Turbo",
        dir: "whisper-turbo",
        company: "OpenAI",
        description: "For the roughly 70 languages the others do not cover. It scores 9.3%, the same as SenseVoice, at three times the size and five times slower than Parakeet on the same audio, because of how the model is built rather than how big it is. Choose it for reach, and only for reach.",
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
];

/// The model that draws words on the overlay while the key is held.
///
/// Deliberately not in `MODELS`. It is not a choice between transcription
/// engines and must never appear next to them: nobody picking a *model* wants
/// to reason about a second one running alongside their first. It is reached
/// through one setting, "show words as you speak", and downloaded by the same
/// command as everything else through `find_any`.
///
/// 73 MB, not the 296 MB the spike quoted. That number was the size of the
/// whole HuggingFace directory, which carries both precisions and two
/// left-context variants; the four files actually needed are a tenth of it.
/// Worth knowing, because the download was most of the argument against
/// building this at all.
pub const STREAMING_MODEL: ModelSpec = ModelSpec {
    id: "streaming-zipformer-en",
    display: "Live preview",
    dir: "streaming-zipformer-en",
    company: "k2-fsa",
    description: "Draws words on the overlay while you are still speaking. English only, no punctuation, and never what gets pasted.",
    size: "73 MB",
    languages: "English",
    hf_base: "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-en-2023-06-26/resolve/main",
    files: &[
        ("encoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx", 71_083_163),
        ("decoder-epoch-99-avg-1-chunk-16-left-128.onnx", 2_092_621),
        ("joiner-epoch-99-avg-1-chunk-16-left-128.int8.onnx", 259_335),
        ("tokens.txt", 5_048),
    ],
    encoder_files: &["encoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx"],
    kind: EngineKind::Streaming,
};

/// The model loaded when the user has expressed no preference.
pub const DEFAULT_MODEL_ID: &str = "parakeet";

pub fn find(id: &str) -> Option<&'static ModelSpec> {
    MODELS.iter().find(|m| m.id == id)
}

/// Like `find`, but also resolves models that are downloadable without being
/// offered in the Models tab. Download and removal go through this; anything
/// that picks a transcription engine must keep using `find`.
pub fn find_any(id: &str) -> Option<&'static ModelSpec> {
    find(id).or(if id == STREAMING_MODEL.id {
        Some(&STREAMING_MODEL)
    } else {
        None
    })
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

#[cfg(test)]
mod streaming_model_tests {
    use super::*;

    /// The whole design of this feature is one setting, not a sixth model. If
    /// it ever reaches the catalogue, the Models tab starts offering a model
    /// that cannot transcribe anything.
    #[test]
    fn the_streaming_model_is_not_a_transcription_choice() {
        assert!(
            find(STREAMING_MODEL.id).is_none(),
            "the streaming model leaked into MODELS"
        );
        let dir = std::env::temp_dir().join("inkwell-streaming-catalog");
        assert!(
            !catalog(&dir).iter().any(|m| m.id == STREAMING_MODEL.id),
            "the streaming model leaked into the UI catalogue"
        );
    }

    /// ...but it still has to be downloadable, which is the trap `find` alone
    /// would spring: `download_model` would answer "Unknown model" for it.
    #[test]
    fn but_it_is_still_downloadable() {
        assert!(find_any(STREAMING_MODEL.id).is_some());
        for m in MODELS {
            assert!(find_any(m.id).is_some(), "{} unreachable via find_any", m.id);
        }
        assert!(find_any("nonsense").is_none());
    }

    #[test]
    fn its_install_check_matches_a_file_it_downloads() {
        let downloaded: Vec<&str> = STREAMING_MODEL.files.iter().map(|(n, _)| *n).collect();
        assert!(
            STREAMING_MODEL
                .encoder_files
                .iter()
                .any(|e| downloaded.contains(e)),
            "the streaming model would install and still report itself missing"
        );
    }

    /// A streaming transducer has no offline decode path. Loading it as a
    /// `SpeechEngine` must fail loudly rather than return something that
    /// produces empty transcripts.
    #[test]
    fn loading_it_as_a_transcription_engine_is_refused() {
        let dir = std::env::temp_dir().join("inkwell-streaming-load");
        // `SpeechEngine` is not Debug, so unwrap_err is unavailable here.
        let Err(err) = STREAMING_MODEL.load(&dir) else {
            panic!("the streaming model loaded as a transcription engine");
        };
        assert!(err.contains("streaming"), "unhelpful refusal: {}", err);
    }

    /// The size string is what the user reads before agreeing to the download,
    /// so it must not drift from the bytes actually fetched.
    #[test]
    fn the_advertised_size_matches_the_files() {
        let mb = STREAMING_MODEL.total_bytes() / 1_000_000;
        assert_eq!(STREAMING_MODEL.size, format!("{} MB", mb));
    }

    #[test]
    fn it_does_not_collide_with_a_real_model_directory() {
        assert!(
            !MODELS.iter().any(|m| m.dir == STREAMING_MODEL.dir),
            "sharing a directory would make removal delete the wrong model"
        );
    }
}

#[cfg(test)]
mod hotword_lifecycle_tests {
    use super::*;

    #[test]
    fn only_construction_time_models_claim_to_need_a_rebuild() {
        // If this ever returns true for a per-utterance model, every dictionary
        // edit would rebuild an engine for no reason, which is a multi-second
        // stall on a keystroke.
        for m in MODELS {
            let expected = matches!(m.kind, EngineKind::Qwen3);
            assert_eq!(
                m.hotwords_at_load(),
                expected,
                "{} disagrees about needing a rebuild",
                m.id
            );
        }
    }

    #[test]
    fn writing_hotwords_is_a_no_op_for_per_utterance_models() {
        // It must not create stray files next to models that will never read
        // them, and must not fail when the model directory does not exist.
        let dir = std::env::temp_dir().join("inkwell-hotword-noop");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let parakeet = find("parakeet").unwrap();
        assert!(parakeet.write_hotwords(&dir, Some("Inkwell")).is_ok());
        assert!(!dir.join(parakeet.dir).join("hotwords.txt").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_empty_dictionary_writes_an_empty_file_rather_than_none() {
        // "No hotwords" and "never written" look identical if the file is
        // deleted, and the difference is the first thing worth checking when a
        // name is still misheard.
        let dir = std::env::temp_dir().join("inkwell-hotword-empty");
        let _ = std::fs::remove_dir_all(&dir);
        let Some(q) = find("qwen3-asr") else { return };
        std::fs::create_dir_all(dir.join(q.dir)).unwrap();
        q.write_hotwords(&dir, None).unwrap();
        let p = dir.join(q.dir).join("hotwords.txt");
        assert!(p.exists());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hotwords_round_trip_to_the_file_the_engine_reads() {
        let dir = std::env::temp_dir().join("inkwell-hotword-write");
        let _ = std::fs::remove_dir_all(&dir);
        let Some(q) = find("qwen3-asr") else { return };
        std::fs::create_dir_all(dir.join(q.dir)).unwrap();
        q.write_hotwords(&dir, Some("Inkwell\nVercel")).unwrap();
        let got = std::fs::read_to_string(dir.join(q.dir).join("hotwords.txt")).unwrap();
        assert_eq!(got, "Inkwell\nVercel");
        std::fs::remove_dir_all(&dir).ok();
    }
}
