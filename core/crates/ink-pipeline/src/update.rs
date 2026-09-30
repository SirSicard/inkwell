//! Updating a model: unload it through residency first, then replace its files.
//!
//! A loaded model holds its files open (memory-mapped weights), and Windows refuses to replace or
//! delete an open file. So the order is fixed, and nothing is written while a model is loaded:
//!
//! 1. **Unload** the model being replaced ([`Residency::unload`], which also stops keeping it
//!    warm), and any model already loaded under the new id, whose files the install could also
//!    write. Refused while a job holds either one ([`UpdateError::StillLoaded`]).
//! 2. **Confirm** they are gone. A model residency had loaded but now reports as not loaded means
//!    the caller and residency disagree about which model this is: a wrong id must not let an
//!    update write under a live model, so that stops the update ([`UpdateError::Mismatch`]).
//! 3. **Install** the new row.
//! 4. **Warm** the new model if the old one was warm.
//!
//! When the update stops after step 1 unloaded the warm model, the old model is warmed again, and
//! if that fails too the error says so ([`UpdateError::no_model_warm`]).
//!
//! **Caller's part:** unloading does not keep a model out; a job that asks for it loads it again.
//! Run an update where no dictation or meeting job can start meanwhile (on the thread that owns
//! them, or with them paused).

use std::fmt;

use ink_core::{CancelToken, EngineError, EventSink};
use ink_engines::{DownloadError, DownloadProgress, Downloader, EngineRow, Residency, Unloaded};

/// What an update needs from residency. [`Residency`] is the real one.
pub trait ModelResidency: Send + Sync {
    /// The id of the model kept warm.
    fn warm(&self) -> Option<String>;

    /// **Worker.** Keeps `row`'s model warm, loading it; `None` keeps none warm.
    fn set_warm(&self, row: Option<&EngineRow>) -> Result<(), EngineError>;

    /// Whether `id` is loaded.
    fn is_resident(&self, id: &str) -> bool;

    /// **Worker.** Unloads `id` now and stops keeping it warm, or refuses (and changes nothing)
    /// while a job holds it. Says whether it was loaded.
    fn unload(&self, id: &str) -> Result<Unloaded, EngineError>;
}

/// What an update needs to install a row's files.
pub trait ModelInstaller: Send + Sync {
    /// **Worker.** Installs `row`, blocking until it is verified and in place. `progress` runs on
    /// this thread, as the transfer goes, and must not block.
    fn install(
        &self,
        row: &EngineRow,
        cancel: &CancelToken,
        progress: EventSink<DownloadProgress>,
    ) -> Result<(), DownloadError>;
}

impl ModelInstaller for Downloader {
    fn install(
        &self,
        row: &EngineRow,
        cancel: &CancelToken,
        progress: EventSink<DownloadProgress>,
    ) -> Result<(), DownloadError> {
        self.download(row, cancel, progress)
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

    fn unload(&self, id: &str) -> Result<Unloaded, EngineError> {
        Residency::unload(self, id)
    }
}

/// Why an update stopped. Every variant says whether a model is left warm
/// ([`no_model_warm`](Self::no_model_warm)), so the shell can tell the user that dictation needs
/// a model before the next take finds out.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum UpdateError {
    /// A model could not be unloaded (a job holds it). Nothing on disk changed.
    StillLoaded {
        /// Why it could not be unloaded.
        error: EngineError,
        /// Set when it was warm and could not be loaded again: no model is warm.
        rewarm_failed: Option<EngineError>,
    },
    /// Residency had the model loaded, then reported it as not loaded: the caller and residency
    /// disagree about which model this is. Nothing on disk changed.
    Mismatch {
        /// The id the update was told to replace.
        id: String,
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
            Self::StillLoaded { rewarm_failed, .. }
            | Self::Mismatch { rewarm_failed, .. }
            | Self::Install { rewarm_failed, .. } => rewarm_failed.is_some(),
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
            Self::Mismatch { id, rewarm_failed } => {
                write!(
                    f,
                    "model {id} was loaded but residency reported it not loaded; nothing was replaced"
                )?;
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

/// **Worker.** Replaces `current` with `next` in the order the module docs give. The install's
/// progress goes to `progress`, on this thread.
pub fn update_model(
    residency: &dyn ModelResidency,
    installer: &dyn ModelInstaller,
    current: &EngineRow,
    next: &EngineRow,
    cancel: &CancelToken,
    progress: EventSink<DownloadProgress>,
) -> Result<(), UpdateError> {
    let was_warm = residency.warm().as_deref() == Some(current.id.as_str());
    // Warms the previous model again once the update has stopped, if the update un-warmed it; its
    // failure goes into the error.
    let rewarm_current = || {
        let lost = was_warm && residency.warm().as_deref() != Some(current.id.as_str());
        if lost {
            residency.set_warm(Some(current)).err()
        } else {
            None
        }
    };

    // 1 and 2: unload, and confirm. `current` first; then a model under the new id, if other.
    let mut ids = vec![current.id.as_str()];
    if next.id != current.id {
        ids.push(next.id.as_str());
    }
    for id in ids {
        let expected = residency.is_resident(id);
        match residency.unload(id) {
            Err(error) => {
                return Err(UpdateError::StillLoaded {
                    error,
                    rewarm_failed: rewarm_current(),
                });
            }
            Ok(Unloaded::NotLoaded) if expected => {
                return Err(UpdateError::Mismatch {
                    id: id.to_owned(),
                    rewarm_failed: rewarm_current(),
                });
            }
            Ok(Unloaded::WasLoaded | Unloaded::NotLoaded) => {}
        }
        if residency.is_resident(id) {
            return Err(UpdateError::StillLoaded {
                error: EngineError::Failed(format!("model {id} is still loaded after unloading")),
                rewarm_failed: rewarm_current(),
            });
        }
    }

    // 3.
    if let Err(error) = installer.install(next, cancel, progress) {
        return Err(UpdateError::Install {
            error,
            rewarm_failed: rewarm_current(),
        });
    }
    // 4.
    if was_warm && let Err(error) = residency.set_warm(Some(next)) {
        return Err(UpdateError::Warm {
            id: next.id.clone(),
            error,
        });
    }
    Ok(())
}
