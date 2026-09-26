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

/// Why an update stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum UpdateError {
    /// The model could not be unloaded. Nothing on disk changed and it is warm again if it was.
    StillLoaded(EngineError),
    /// The new files could not be installed. The old model is warm again if it was (a failure to
    /// warm it is logged).
    Install(DownloadError),
    /// Installed, but the new model failed to load.
    Warm(EngineError),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StillLoaded(e) => write!(f, "the model could not be unloaded: {e}"),
            Self::Install(e) => write!(f, "the new model could not be installed: {e}"),
            Self::Warm(e) => write!(f, "the new model was installed but did not load: {e}"),
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
    let rewarm_current = || {
        if was_warm && let Err(e) = residency.set_warm(Some(current)) {
            log::error!("model update: the previous model could not be loaded again: {e}");
        }
    };
    if was_warm {
        residency.set_warm(None).map_err(UpdateError::StillLoaded)?;
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
    if let Err(e) = unloaded {
        rewarm_current();
        return Err(UpdateError::StillLoaded(e));
    }
    if let Err(e) = installer.install(next, cancel) {
        rewarm_current();
        return Err(UpdateError::Install(e));
    }
    if was_warm {
        residency.set_warm(Some(next)).map_err(UpdateError::Warm)?;
    }
    Ok(())
}
