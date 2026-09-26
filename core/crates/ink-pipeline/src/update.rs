//! Updating a model: unload it through residency first, then replace its files.
//!
//! A loaded model holds its files open (memory-mapped weights), and Windows refuses to replace or
//! delete an open file. So the order is fixed, and nothing is written while the model is loaded:
//!
//! 1. stop keeping it warm;
//! 2. unload it, and confirm it is gone. If it cannot be unloaded, stop: nothing on disk changes,
//!    and it is warm again ([`UpdateError::StillLoaded`]);
//! 3. install the new row;
//! 4. warm the new model if the old one was warm. A failed install warms the old one again.
//!
//! **Gap, reported:** `ink-engines`' [`Residency`] has no way to unload a model on demand; it
//! unloads a model only after five idle minutes, and never the warm one. Its implementation of
//! [`ModelResidency::unload`] therefore unloads when `tick` can, and otherwise refuses with
//! [`EngineError::Unsupported`], so today an update of a loaded model fails closed rather than
//! writing under it. The smallest fix is one method on `Residency`: unload one id now, refusing
//! while a lease holds it.

use std::fmt;
use std::sync::Arc;

use ink_core::{CancelToken, EngineError};
use ink_engines::{DownloadError, Downloader, EngineRow, Residency};

/// What an update needs from residency.
pub trait ModelResidency: Send + Sync {
    /// The id of the model kept warm.
    fn warm(&self) -> Option<String>;

    /// **Worker.** Keeps `row`'s model warm, loading it; `None` keeps none warm.
    fn set_warm(&self, row: Option<&EngineRow>) -> Result<(), EngineError>;

    /// Whether `id` is loaded.
    fn is_resident(&self, id: &str) -> bool;

    /// **Worker.** Unloads `id` now, or refuses. When it returns `Ok`, no copy of the model is
    /// loaded and its files are closed.
    fn unload(&self, id: &str) -> Result<(), EngineError>;
}

/// What an update needs to install a row's files.
pub trait ModelInstaller: Send + Sync {
    /// **Worker.** Installs `row`, blocking until it is verified and in place.
    fn install(&self, row: &EngineRow, cancel: &CancelToken) -> Result<(), DownloadError>;
}

impl ModelInstaller for Downloader {
    fn install(&self, row: &EngineRow, cancel: &CancelToken) -> Result<(), DownloadError> {
        // Progress is the shell's to show through its own download call; an update reports only
        // how it ended.
        self.download(row, cancel, Arc::new(|_| {}))
    }
}

impl<M: Send + Sync + 'static> ModelResidency for Residency<M> {
    fn warm(&self) -> Option<String> {
        Residency::warm(self)
    }

    fn set_warm(&self, row: Option<&EngineRow>) -> Result<(), EngineError> {
        Residency::set_warm(self, row)
    }

    fn is_resident(&self, id: &str) -> bool {
        self.resident().iter().any(|r| r == id)
    }

    fn unload(&self, id: &str) -> Result<(), EngineError> {
        if !self.is_resident(id) {
            return Ok(());
        }
        self.tick();
        if self.is_resident(id) {
            return Err(EngineError::Unsupported(
                "unloading a model on demand (residency unloads only after five idle minutes)",
            ));
        }
        Ok(())
    }
}

/// Why an update stopped. Every variant says whether a model is left warm
/// ([`no_model_warm`](Self::no_model_warm)), so the shell can tell the user that dictation needs
/// a model before the next take finds out.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum UpdateError {
    /// The model could not be unloaded. Nothing on disk changed.
    StillLoaded {
        /// Why it could not be unloaded.
        error: EngineError,
        /// Set when it was warm and could not be loaded again: no model is warm.
        rewarm_failed: Option<EngineError>,
    },
    /// The new files could not be installed.
    Install {
        /// Why.
        error: DownloadError,
        /// Set when the previous model was warm and could not be loaded again: no model is warm.
        rewarm_failed: Option<EngineError>,
    },
    /// The new model was installed but did not load. No model is warm.
    Warm {
        /// The new model's registry id, for the shell to name.
        id: String,
        /// Why it did not load.
        error: EngineError,
    },
}

impl UpdateError {
    /// Whether the update left no model warm: dictation has no model until one is loaded.
    pub fn no_model_warm(&self) -> bool {
        match self {
            Self::StillLoaded { rewarm_failed, .. } | Self::Install { rewarm_failed, .. } => {
                rewarm_failed.is_some()
            }
            Self::Warm { .. } => true,
        }
    }
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rewarm = |f: &mut fmt::Formatter<'_>, failed: &Option<EngineError>| match failed {
            Some(e) => write!(f, "; the previous model did not load again either: {e}"),
            None => Ok(()),
        };
        match self {
            Self::StillLoaded {
                error,
                rewarm_failed,
            } => {
                write!(f, "the model could not be unloaded: {error}")?;
                rewarm(f, rewarm_failed)
            }
            Self::Install {
                error,
                rewarm_failed,
            } => {
                write!(f, "the new model could not be installed: {error}")?;
                rewarm(f, rewarm_failed)
            }
            Self::Warm { id, error } => {
                write!(f, "model {id} was installed but did not load: {error}")
            }
        }
    }
}

impl std::error::Error for UpdateError {}

/// **Worker.** Replaces `current` with `next` in the order the module docs give.
pub fn update_model(
    residency: &dyn ModelResidency,
    installer: &dyn ModelInstaller,
    current: &EngineRow,
    next: &EngineRow,
    cancel: &CancelToken,
) -> Result<(), UpdateError> {
    let was_warm = residency.warm().as_deref() == Some(current.id.as_str());
    // Warms the previous model again after a failed update; its failure goes into the error.
    let rewarm_current = || {
        if was_warm {
            residency.set_warm(Some(current)).err()
        } else {
            None
        }
    };
    if was_warm && let Err(error) = residency.set_warm(None) {
        return Err(UpdateError::StillLoaded {
            error,
            rewarm_failed: None,
        });
    }
    let unloaded = residency.unload(&current.id).and_then(|()| {
        // Trust, but check: an update must never write under a loaded model.
        if residency.is_resident(&current.id) {
            Err(EngineError::Failed(
                "the model is still loaded after unloading it".into(),
            ))
        } else {
            Ok(())
        }
    });
    if let Err(error) = unloaded {
        return Err(UpdateError::StillLoaded {
            error,
            rewarm_failed: rewarm_current(),
        });
    }
    if let Err(error) = installer.install(next, cancel) {
        return Err(UpdateError::Install {
            error,
            rewarm_failed: rewarm_current(),
        });
    }
    if was_warm && let Err(error) = residency.set_warm(Some(next)) {
        return Err(UpdateError::Warm {
            id: next.id.clone(),
            error,
        });
    }
    Ok(())
}
