//! Registry rows: what a model is, where its files come from, and what it is good at.
//!
//! Rows are data. Adding a model means adding a row to [`builtin_rows`]; the router, downloader
//! and residency never name a model. Every row is validated before anything uses it
//! ([`Registry::new`], and again by the downloader), so a row that would download from a moving
//! branch, fetch weights under a licence the project does not allow, or write outside the model
//! directory is refused up front with an error that names the row.

use std::collections::HashSet;
use std::fmt;

use ink_core::{EngineInfo, Job};

use crate::model_dir::{MAX_RELATIVE_PATH_LEN, PART_SUFFIX, REVISION_DIR_LEN};

/// Licences model weights may carry (docs/MODEL-WEIGHTS.md). Anything else, including the NVIDIA
/// Open Model Licence and Hugging Face's `other`, needs a policy change before a row can use it.
pub const ALLOWED_WEIGHT_LICENCES: &[&str] = &["Apache-2.0", "MIT", "OpenMDW-1.1", "CC-BY-4.0"];

/// Longest id, or component of a file name, a row may use. A row's paths are also checked whole
/// against [`MAX_RELATIVE_PATH_LEN`](crate::MAX_RELATIVE_PATH_LEN) below the root; a file name of
/// one component at this limit always fits.
pub const MAX_NAME_LEN: usize = 64;

/// An operating system the core ships on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Os {
    /// macOS.
    MacOs,
    /// Windows.
    Windows,
}

impl Os {
    /// The OS this build targets, or `None` on one the core does not ship on (the router then
    /// finds no engine rather than guessing).
    pub const fn current() -> Option<Os> {
        if cfg!(target_os = "macos") {
            Some(Os::MacOs)
        } else if cfg!(target_os = "windows") {
            Some(Os::Windows)
        } else {
            None
        }
    }
}

/// Which adapter loads a row's files. A new runtime is code (an adapter behind a cargo feature);
/// a new model for an existing runtime is only a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Runtime {
    /// llama.cpp, for GGUF speech and language models (`engine-llama`).
    LlamaCpp,
    /// sherpa-onnx, for ONNX speech models.
    SherpaOnnx,
    /// NeMo-Speech.cpp, for the diarizer (`engine-nemo`).
    NemoSpeechCpp,
    /// tract, a pure-Rust ONNX runtime, for Silero VAD (`engine-silero`).
    Tract,
    /// Apple's Core ML, run by the Mac shell (FluidAudio), never by the core (architecture rule
    /// 2). The core only downloads such a row: it fills no job, so the router never picks it and
    /// residency never loads it; the shell loads the files and registers its own engine.
    CoreMl,
}

impl Runtime {
    /// Whether its speech models take seconds for a dictation on a machine without a GPU, so the
    /// router gives dictation there to an installed engine that does not ([`Router`]). llama.cpp's
    /// do: on a 12-core desktop's CPU, Qwen3-ASR 1.7B took about 1.6-1.9 s for 5 s of speech, and
    /// Parakeet on sherpa-onnx about 0.3 s (not a measurement: other builds shared the machine).
    ///
    /// [`Router`]: crate::Router
    pub fn slow_on_cpu_for_dictation(self) -> bool {
        matches!(self, Self::LlamaCpp)
    }
}

/// One job a row can fill, with its measured error rate on that job's benchmark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JobScore {
    /// The job.
    pub job: Job,
    /// Measured error rate in percent, lower is better: word error rate for the speech jobs,
    /// diarization error rate for [`Job::Diarization`], and for [`Job::VoiceActivity`] the share
    /// of clearly speech or clearly silent windows it misjudges. Per job, because one model is
    /// measured on a different set for each job (meetings versus dictation), and the router only
    /// compares numbers measured for the same job.
    pub wer: f32,
}

/// One file of a model, pinned by hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelFile {
    /// The file's path inside the row's directory: one name, or names separated by `/` for a file
    /// in a subdirectory (a Core ML model is a directory of files). Each name is ASCII letters,
    /// digits, `-`, `_` and `.`, starting with a letter or digit, so the path is never absolute,
    /// never climbs with `..`, and cannot leave that directory.
    pub name: String,
    /// Where to fetch it: `https`, with the row's [`revision`](EngineRow::revision) as a path
    /// segment (for Hugging Face, `/resolve/<revision>/`).
    pub url: String,
    /// SHA-256 of the whole file, 64 lowercase hex digits.
    pub sha256: String,
    /// Size in bytes.
    pub size: u64,
}

/// A downloadable model and what it does.
#[derive(Clone, Debug, PartialEq)]
pub struct EngineRow {
    /// Stable id, also the directory name (for example `qwen3-asr-1.7b-q8`).
    pub id: String,
    /// The jobs it fills, each with its measured error rate.
    pub scores: Vec<JobScore>,
    /// Its files.
    pub files: Vec<ModelFile>,
    /// The exact repository revision every URL is pinned to: a full commit hash (40 or 64
    /// lowercase hex digits), never a branch or tag, which can move under a published hash.
    pub revision: String,
    /// The weights' licence, SPDX where one exists. Must be in [`ALLOWED_WEIGHT_LICENCES`].
    pub licence: String,
    /// The OSes it runs on.
    pub oses: Vec<Os>,
    /// The adapter that loads it.
    pub runtime: Runtime,
    /// What the model is for: speech jobs, or language (polish, voice edit, summaries, Ask).
    pub kind: RowKind,
}

/// What a row's model is for.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RowKind {
    /// A speech model (or the VAD, or the diarizer): it fills the jobs its scores name, and the
    /// router picks among such rows.
    Speech,
    /// A language model the core runs itself (llama.cpp, `engine-llama`): polish, voice edit, a
    /// meeting's summary and Ask, on this machine. It fills no speech job, so the router never
    /// picks it; the core keeps it apart from the speech models and loads it on demand.
    Language(LanguageRow),
}

/// What a language row adds to a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanguageRow {
    /// Its name for the user, as `models.listed` gives it (for example `Qwen3 4B Instruct`).
    pub name: String,
    /// What its chat format needs beyond llama.cpp's built-in template.
    pub chat: ChatQuirks,
}

/// What a language model's chat format needs beyond llama.cpp's built-in template, which the
/// adapter applies (it has no Jinja, so a template's own switches cannot be passed).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ChatQuirks {
    /// A hybrid thinking model (Qwen3's 1.7B and 8B): its thinking is turned off the way its own
    /// template does with `enable_thinking=false`, by starting the answer with an empty think
    /// block ([`chat::NO_THINK_PREFILL`](crate::chat::NO_THINK_PREFILL)). No row in 1.0 needs it:
    /// Qwen3-4B-Instruct-2507 never thinks.
    pub no_think: bool,
}

impl EngineRow {
    /// Whether this row fills `job`.
    pub fn does(&self, job: Job) -> bool {
        self.scores.iter().any(|s| s.job == job)
    }

    /// The measured error rate for `job`, if this row fills it.
    pub fn wer(&self, job: Job) -> Option<f32> {
        self.scores.iter().find(|s| s.job == job).map(|s| s.wer)
    }

    /// Whether this row runs on `os`.
    pub fn runs_on(&self, os: Os) -> bool {
        self.oses.contains(&os)
    }

    /// The sum of its files' sizes.
    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// What this engine is, for the About screen and the shell.
    pub fn info(&self) -> EngineInfo {
        EngineInfo {
            id: self.id.clone(),
            jobs: self.scores.iter().map(|s| s.job).collect(),
            licence: self.licence.clone(),
        }
    }

    /// Checks this row on its own: everything [`Registry::new`] checks except uniqueness across
    /// rows.
    pub fn validate(&self) -> Result<(), RegistryError> {
        let invalid = |reason: String| RegistryError::Invalid {
            id: self.id.clone(),
            reason,
        };
        check_name(&self.id).map_err(|why| invalid(format!("id {:?} {why}", self.id)))?;
        if !is_commit_hash(&self.revision) {
            return Err(RegistryError::Unpinned {
                id: self.id.clone(),
                reason: format!(
                    "revision {:?} is not a full commit hash (40 or 64 lowercase hex digits)",
                    self.revision
                ),
            });
        }
        if !ALLOWED_WEIGHT_LICENCES.contains(&self.licence.as_str()) {
            return Err(RegistryError::Licence {
                id: self.id.clone(),
                licence: self.licence.clone(),
            });
        }
        match (&self.kind, self.runtime, self.scores.is_empty()) {
            (RowKind::Language(_), _, false) => {
                return Err(invalid(
                    "is a language model, so it fills no speech job (the router never picks it)"
                        .into(),
                ));
            }
            (RowKind::Language(_), Runtime::LlamaCpp, true) => {}
            (RowKind::Language(_), _, true) => {
                return Err(invalid(
                    "is a language model, which only the llama.cpp adapter runs".into(),
                ));
            }
            (RowKind::Speech, Runtime::CoreMl, false) => {
                return Err(invalid(
                    "is only downloaded (the shell runs Core ML), so it may fill no job".into(),
                ));
            }
            (RowKind::Speech, Runtime::CoreMl, true) => {}
            (RowKind::Speech, _, true) => return Err(invalid("fills no job".into())),
            (RowKind::Speech, _, false) => {}
        }
        if let RowKind::Language(language) = &self.kind {
            let name = language.name.trim();
            if name.is_empty() || name != language.name {
                return Err(invalid(format!(
                    "language model name {:?} is blank or has spaces at an end",
                    language.name
                )));
            }
            if language.name.chars().count() > MAX_NAME_LEN
                || language.name.chars().any(char::is_control)
            {
                return Err(invalid(format!(
                    "language model name {:?} is over {MAX_NAME_LEN} characters or holds a \
                     control character",
                    language.name
                )));
            }
            // The adapter loads one GGUF file.
            if !(self.files.len() == 1 && self.files[0].name.ends_with(".gguf")) {
                return Err(invalid(
                    "is a language model, which is exactly one .gguf file".into(),
                ));
            }
        }
        for (i, score) in self.scores.iter().enumerate() {
            // NaN would make the router's ordering meaningless; a negative rate is a typo.
            if !score.wer.is_finite() || score.wer < 0.0 {
                return Err(invalid(format!(
                    "error rate {} for {:?} is not a finite, non-negative number",
                    score.wer, score.job
                )));
            }
            if self.scores[..i].iter().any(|s| s.job == score.job) {
                return Err(invalid(format!("scores {:?} twice", score.job)));
            }
        }
        if self.oses.is_empty() {
            return Err(invalid("runs on no OS".into()));
        }
        if self.files.is_empty() {
            return Err(invalid("has no files".into()));
        }
        for (i, file) in self.files.iter().enumerate() {
            for part in file.name.split('/') {
                check_name(part)
                    .map_err(|why| invalid(format!("file name {:?}: {part:?} {why}", file.name)))?;
                if part.ends_with(PART_SUFFIX) {
                    return Err(invalid(format!(
                        "file name {:?} ends a name in {PART_SUFFIX}, which downloads in progress use",
                        file.name
                    )));
                }
            }
            // `<id>/<revision prefix>/<name>.part`, the longest path the row writes.
            let longest =
                self.id.len() + 1 + REVISION_DIR_LEN + 1 + file.name.len() + PART_SUFFIX.len();
            if longest > MAX_RELATIVE_PATH_LEN {
                return Err(invalid(format!(
                    "file name {:?} makes a path of {longest} characters below the model root, over {MAX_RELATIVE_PATH_LEN}",
                    file.name
                )));
            }
            if self.files[..i].iter().any(|f| f.name == file.name) {
                return Err(invalid(format!("file name {:?} appears twice", file.name)));
            }
            // A name cannot be both a file and the directory of another.
            if let Some(below) = self.files.iter().find(|f| {
                f.name
                    .strip_prefix(file.name.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
            }) {
                return Err(invalid(format!(
                    "file name {:?} is also the directory of {:?}",
                    file.name, below.name
                )));
            }
            if file.size == 0 {
                return Err(invalid(format!("file {} has size 0", file.name)));
            }
            if !(file.sha256.len() == 64 && file.sha256.bytes().all(is_lower_hex)) {
                return Err(invalid(format!(
                    "file {} has sha256 {:?}, not 64 lowercase hex digits",
                    file.name, file.sha256
                )));
            }
            check_url(&file.url, &self.revision).map_err(|problem| match problem {
                UrlProblem::NotHttps => invalid(format!("URL {} is not https", file.url)),
                UrlProblem::Unpinned(reason) => RegistryError::Unpinned {
                    id: self.id.clone(),
                    reason,
                },
            })?;
        }
        Ok(())
    }
}

fn is_lower_hex(b: u8) -> bool {
    b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
}

/// A full git commit hash: SHA-1 (40 digits) or SHA-256 (64). A short hash is refused too: it is
/// pinned today and ambiguous once the repository grows.
fn is_commit_hash(revision: &str) -> bool {
    matches!(revision.len(), 40 | 64) && revision.bytes().all(is_lower_hex)
}

/// Ids and file names become path components, so only a conservative set is allowed: ASCII
/// letters, digits, `-`, `_` and `.`, starting with a letter or digit (no `..`, no hidden files,
/// no separators or drive letters), not ending in `.`, and not a name Windows reserves for a
/// device.
fn check_name(name: &str) -> Result<(), &'static str> {
    const RESERVED: &[&str] = &["con", "prn", "aux", "nul"];
    let first_ok = name
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphanumeric());
    if !first_ok {
        return Err("must start with an ASCII letter or digit");
    }
    if name.len() > MAX_NAME_LEN {
        return Err("is too long");
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err("may only use ASCII letters, digits, '-', '_' and '.'");
    }
    if name.ends_with('.') {
        // Windows drops a trailing dot, so the file on disk would not have this name.
        return Err("may not end in '.'");
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_lowercase();
    let device = RESERVED.contains(&stem.as_str())
        || ((stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if device {
        return Err("is a device name on Windows");
    }
    Ok(())
}

enum UrlProblem {
    NotHttps,
    Unpinned(String),
}

/// The URL must be https and name `revision` as a whole path segment; if it has a Hugging Face
/// style `resolve` segment, the segment after it must be the revision (so `/resolve/main/` is
/// refused even when the revision appears elsewhere in the path).
fn check_url(url: &str, revision: &str) -> Result<(), UrlProblem> {
    let rest = url.strip_prefix("https://").ok_or(UrlProblem::NotHttps)?;
    let path = rest
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .split_once('/')
        .map_or("", |(_host, path)| path);
    let segments: Vec<&str> = path.split('/').collect();
    for (i, segment) in segments.iter().enumerate() {
        if *segment == "resolve" {
            let resolved = segments.get(i + 1).copied().unwrap_or_default();
            if resolved != revision {
                return Err(UrlProblem::Unpinned(format!(
                    "URL {url} resolves {resolved:?}, not the pinned revision {revision}"
                )));
            }
        }
    }
    if !segments.contains(&revision) {
        return Err(UrlProblem::Unpinned(format!(
            "URL {url} does not name the pinned revision {revision}"
        )));
    }
    Ok(())
}

/// Why a row was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegistryError {
    /// The revision is not a full commit hash, or a URL is not pinned to it (for example
    /// `/resolve/main`).
    Unpinned {
        /// The row.
        id: String,
        /// What is wrong, naming the revision or URL.
        reason: String,
    },
    /// The weights' licence is not in [`ALLOWED_WEIGHT_LICENCES`].
    Licence {
        /// The row.
        id: String,
        /// The refused licence.
        licence: String,
    },
    /// Two rows share an id.
    Duplicate {
        /// The id.
        id: String,
    },
    /// Anything else malformed: an empty or unsafe id or file name, a bad hash, a zero size, no
    /// jobs (or, for a language model, any), a non-finite error rate, no OS.
    Invalid {
        /// The row.
        id: String,
        /// What is wrong.
        reason: String,
    },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unpinned { id, reason } => write!(f, "registry row {id}: not pinned: {reason}"),
            Self::Licence { id, licence } => {
                write!(
                    f,
                    "registry row {id}: weights licence {licence} is not allowed"
                )
            }
            Self::Duplicate { id } => write!(f, "registry row {id}: duplicate id"),
            Self::Invalid { id, reason } => write!(f, "registry row {id}: {reason}"),
        }
    }
}

impl std::error::Error for RegistryError {}

/// A validated set of rows.
#[derive(Clone, Debug, Default)]
pub struct Registry {
    rows: Vec<EngineRow>,
}

impl Registry {
    /// Validates every row and their ids' uniqueness. The first problem found is returned.
    pub fn new(rows: Vec<EngineRow>) -> Result<Self, RegistryError> {
        let mut seen = HashSet::new();
        for row in &rows {
            row.validate()?;
            if !seen.insert(row.id.as_str()) {
                return Err(RegistryError::Duplicate { id: row.id.clone() });
            }
        }
        Ok(Self { rows })
    }

    /// The rows the app ships with.
    pub fn builtin() -> Result<Self, RegistryError> {
        Self::new(builtin_rows())
    }

    /// Every row, in the order given.
    pub fn rows(&self) -> &[EngineRow] {
        &self.rows
    }

    /// The row with this id.
    pub fn get(&self, id: &str) -> Option<&EngineRow> {
        self.rows.iter().find(|r| r.id == id)
    }
}

/// The models the app ships knowing about.
///
/// Each model's revision, hashes and sizes are confirmed when its adapter lands, and its row is
/// only added then. Error rates are measured per job on the same sets for every row, so the router
/// compares like with like: the meeting final and live partials on AMI IHM (three public meeting
/// excerpts, 709 reference words), the dictation final on FLEURS English dev as published (394
/// utterances). The diarizer, the VAD, Windows' Parakeet and the language model are listed only in
/// builds that include their adapter, so such a build never offers a download it cannot run. The
/// Mac's Parakeet is in every build: the shell runs it.
pub fn builtin_rows() -> Vec<EngineRow> {
    [
        qwen3_asr_1_7b_q8(),
        crate::rows::parakeet_tdt_v3_coreml(),
        #[cfg(feature = "engine-nemo")]
        crate::rows::nemotron_3_diarization(),
        #[cfg(feature = "engine-silero")]
        crate::rows::silero_vad(),
        #[cfg(feature = "engine-sherpa")]
        crate::rows::parakeet_tdt_v3_int8(),
        #[cfg(feature = "engine-llama")]
        crate::rows::qwen3_4b_instruct_2507_q4km(),
    ]
    .into_iter()
    .collect()
}

/// Qwen3-ASR 1.7B, Q8_0 GGUF plus its Q8_0 audio projector, for llama.cpp (`engine-llama`).
///
/// Licence: the base model, `Qwen/Qwen3-ASR-1.7B`, is Apache-2.0 on its model card (checked
/// 2026-09-26). The `ggml-org/Qwen3-ASR-1.7B-GGUF` conversion's repository carries no licence tag
/// (its files' own metadata say `apache-2.0`), so the row records the base model's licence.
///
/// Sizes and hashes are those of the files downloaded from this revision (checked locally by an
/// ignored test). Windows too: on a desktop's Vulkan GPU it matched the Mac's WER and ran faster;
/// on the CPU alone its WER holds, and it fits a meeting's final pass, but a dictation takes
/// seconds.
fn qwen3_asr_1_7b_q8() -> EngineRow {
    const REVISION: &str = "36a678687ba7d07a74ca70ccb0e36902e005fb80";
    let file = |name: &str, sha256: &str, size: u64| ModelFile {
        name: name.into(),
        url: format!(
            "https://huggingface.co/ggml-org/Qwen3-ASR-1.7B-GGUF/resolve/{REVISION}/{name}"
        ),
        sha256: sha256.into(),
        size,
    };
    EngineRow {
        id: "qwen3-asr-1.7b-q8".into(),
        scores: vec![
            JobScore {
                job: Job::MeetingFinal,
                wer: 16.08,
            },
            JobScore {
                job: Job::DictationFinal,
                wer: 4.59,
            },
        ],
        files: vec![
            file(
                "Qwen3-ASR-1.7B-Q8_0.gguf",
                "58e22d0532d4eacaf034cfac17a6fed159f37c41390c710186783be439d1fc57",
                2_165_034_944,
            ),
            file(
                "mmproj-Qwen3-ASR-1.7B-Q8_0.gguf",
                "46c1d533af3f354ceb37ce855dbceff7da7fa7cf1e6a523df3b13440bd164c0d",
                355_709_344,
            ),
        ],
        revision: REVISION.into(),
        licence: "Apache-2.0".into(),
        oses: vec![Os::MacOs, Os::Windows],
        runtime: Runtime::LlamaCpp,
        kind: RowKind::Speech,
    }
}
