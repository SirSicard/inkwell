//! Errors, one type per trait family.
//!
//! Messages name what failed, never what was said: no transcript text, prompt or audio content
//! goes into an error (I5), because errors end up in logs.

use std::fmt;

use crate::platform::Permission;

/// Why a platform call failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlatformError {
    /// This OS or build cannot do it. The UI shows it as a state; it is never swallowed into a
    /// silent `None`.
    Unsupported(&'static str),
    /// The permission this call needs is not granted.
    PermissionDenied(Permission),
    /// A device vanished, changed format, or refused to start.
    Device(String),
    /// Anything else, with enough context to act on.
    Failed(String),
}

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(what) => write!(f, "not supported here: {what}"),
            Self::PermissionDenied(p) => write!(f, "permission not granted: {p:?}"),
            Self::Device(msg) => write!(f, "audio device: {msg}"),
            Self::Failed(msg) => write!(f, "platform call failed: {msg}"),
        }
    }
}

impl std::error::Error for PlatformError {}

/// Why an engine call failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EngineError {
    /// The engine does not do this job (for example, streaming on an offline-only engine).
    Unsupported(&'static str),
    /// The model files are not installed, or failed their hash check.
    ModelMissing(String),
    /// The call saw its [`CancelToken`](crate::threading::CancelToken) and stopped.
    Cancelled,
    /// Anything else, with enough context to act on.
    Failed(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(what) => write!(f, "engine does not support: {what}"),
            Self::ModelMissing(model) => write!(f, "model not installed: {model}"),
            Self::Cancelled => f.write_str("cancelled"),
            Self::Failed(msg) => write!(f, "engine failed: {msg}"),
        }
    }
}

impl std::error::Error for EngineError {}

/// Why a store call failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreError {
    /// No record, commitment or setting with that id.
    NotFound,
    /// A supersede with no words. Refused: an engine that returns nothing on audio with speech
    /// would otherwise erase a working transcript.
    EmptySupersede,
    /// A supersede with fewer than half the words of the current revision. Refused as far more
    /// likely an engine failure than a correction.
    SuspiciousSupersede {
        /// Words in the current revision.
        previous_words: usize,
        /// Words in the refused revision.
        new_words: usize,
    },
    /// The request contradicts the store's rules (for example, merging a commitment into itself).
    Invalid(String),
    /// The backing database failed.
    Backend(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("not found"),
            Self::EmptySupersede => f.write_str("refused to supersede with an empty transcript"),
            Self::SuspiciousSupersede {
                previous_words,
                new_words,
            } => write!(
                f,
                "refused to supersede {previous_words} words with {new_words}: more likely an engine failure than a correction"
            ),
            Self::Invalid(msg) => write!(f, "invalid request: {msg}"),
            Self::Backend(msg) => write!(f, "store backend: {msg}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Why a language-model call failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LlmError {
    /// Local-only mode is on and this endpoint is not on this machine (architecture rule 6).
    LocalOnly {
        /// The refused endpoint, for the UI to name.
        endpoint: String,
    },
    /// No API key is stored for the provider.
    NoKey,
    /// The keychain refused access. Nothing was sent.
    KeychainDenied,
    /// The provider answered with an HTTP error status.
    Http {
        /// The status code.
        status: u16,
    },
    /// The request never got an answer.
    Network(String),
    /// The call saw its [`CancelToken`](crate::threading::CancelToken) and stopped.
    Cancelled,
    /// The answer did not have the shape the task asked for.
    BadResponse(String),
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LocalOnly { endpoint } => {
                write!(
                    f,
                    "local-only mode refuses a non-local endpoint: {endpoint}"
                )
            }
            Self::NoKey => f.write_str("no API key stored for this provider"),
            Self::KeychainDenied => f.write_str("keychain access denied; nothing was sent"),
            Self::Http { status } => write!(f, "provider returned HTTP {status}"),
            Self::Network(msg) => write!(f, "network: {msg}"),
            Self::Cancelled => f.write_str("cancelled"),
            Self::BadResponse(msg) => write!(f, "unexpected response: {msg}"),
        }
    }
}

impl std::error::Error for LlmError {}
