//! The models on disk, as the screens manage them: the catalogue's free space and the suggested
//! language model size; a download's free-space check before it fetches anything
//! ([`check_space`]); cancelling a download (`model.cancel`, [`cancel`]) and deleting a model's
//! files (`model.remove`, [`remove`]), both on the queries thread, so neither waits behind a
//! download holding the command thread.

use std::sync::{Arc, Mutex};

use ink_core::CancelToken;
use ink_engines::{EngineRow, Os, RowKind, suggested_language};
use serde_json::Value;

use crate::events::event;
use crate::runtime::{Shared, lock};

/// **Worker.** The bytes free to this user on the volume models are installed on, or `None` when
/// the OS cannot say (logged). Asked of the models root, or, before it exists, of the nearest
/// directory above it that does.
pub fn free_bytes(shared: &Shared) -> Option<u64> {
    let root = shared.models.root();
    let Some(existing) = root.ancestors().find(|p| p.is_dir()) else {
        log::warn!("models: no directory above the models root exists to ask for free space");
        return None;
    };
    match shared.system.free_disk_bytes(existing) {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            log::warn!("models: the free space could not be read ({e})");
            None
        }
    }
}

/// **Worker.** The machine's memory, or `None` when the OS cannot say (logged).
fn memory(shared: &Shared) -> Option<u64> {
    match shared.system.total_memory_bytes() {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            log::warn!("models: the machine's memory could not be read ({e})");
            None
        }
    }
}

/// **Worker.** The language row the core suggests for this machine, if this OS has any: the
/// Default, or the Small one with under 12 GB of memory.
pub fn suggested(shared: &Shared) -> Option<&EngineRow> {
    let os = Os::current()?;
    let rows = shared.registry.rows();
    // Memory is read only where there is a size to suggest (never on the Mac).
    rows.iter()
        .any(|r| r.runs_on(os) && is_language(r))
        .then(|| suggested_language(rows, os, memory(shared)))
        .flatten()
}

/// Whether `row` is a language model.
pub fn is_language(row: &EngineRow) -> bool {
    matches!(row.kind, RowKind::Language(_))
}

/// The free space a download keeps beyond its own bytes: 1 GiB, so a model never fills the disk.
pub const SPACE_MARGIN: u64 = 1 << 30;

/// The download a `model.update` makes, from when it is queued on the command thread until it
/// ends, so `model.cancel` (on the queries thread) reaches it there, or before it starts.
pub struct Install {
    /// The model being replaced (`model.update`'s `model`).
    pub current: String,
    /// The model being downloaded (`next`), the one `model.cancel` names.
    pub next: String,
    /// Its own cancel: the download stops at its next chunk and keeps its part files.
    pub token: CancelToken,
    state: Mutex<InstallState>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InstallState {
    Queued,
    Running,
    /// Cancelled before it started, and answered then.
    Answered,
}

/// The downloads queued or running. `Send + Sync`; the lock guards a short list.
#[derive(Default)]
pub struct Installs {
    list: Mutex<Vec<Arc<Install>>>,
}

impl Installs {
    /// **Any thread.** Registers a `model.update` as it is queued.
    pub fn queue(&self, current: &str, next: &str) -> Arc<Install> {
        let install = Arc::new(Install {
            current: current.to_owned(),
            next: next.to_owned(),
            token: CancelToken::new(),
            state: Mutex::new(InstallState::Queued),
        });
        lock(&self.list).push(install.clone());
        install
    }

    /// **Any thread.** It ended (or was never sent).
    pub fn done(&self, install: &Arc<Install>) {
        lock(&self.list).retain(|i| !Arc::ptr_eq(i, install));
    }

    /// **Any thread.** Cancels every queued or running download of `model`. The running ones stop
    /// at their next chunk and say so when they end; the queued ones are ended now, and returned
    /// for their answer. `None` when there was none.
    fn cancel(&self, model: &str) -> Option<Vec<Arc<Install>>> {
        let list = lock(&self.list);
        let mine: Vec<&Arc<Install>> = list.iter().filter(|i| i.next == model).collect();
        if mine.is_empty() {
            return None;
        }
        let mut answered = Vec::new();
        for install in mine {
            install.token.cancel();
            let mut state = lock(&install.state);
            if *state == InstallState::Queued {
                *state = InstallState::Answered;
                answered.push(install.clone());
            }
        }
        Some(answered)
    }

    /// **Any thread.** Every download's cancel, at shutdown.
    pub fn cancel_all(&self) {
        for install in lock(&self.list).iter() {
            install.token.cancel();
        }
    }
}

impl Install {
    /// **Command thread.** It starts: `false` when a `model.cancel` already ended it (and said so).
    pub fn start(&self) -> bool {
        let mut state = lock(&self.state);
        if *state == InstallState::Answered {
            return false;
        }
        *state = InstallState::Running;
        true
    }
}

/// `model.update_finished` for an update that was cancelled before it started.
pub fn cancelled_event(current: &str, next: &str) -> Value {
    event(
        "model.update_finished",
        &[
            ("id", Some(current.into())),
            ("next", Some(next.into())),
            ("ok", Some(false.into())),
            ("no_model_warm", Some(false.into())),
            ("message", Some("the download was cancelled".into())),
            ("cancelled", Some(true.into())),
        ],
    )
}

/// **Queries thread.** `model.cancel`: stops the download of `model`, running or queued. A running
/// one keeps its part files and answers with its own `model.update_finished` (`cancelled`); a
/// queued one is answered now. Nothing else answers it; no download of it is `not_downloading`.
pub fn cancel(shared: &Shared, model: &str) -> Result<(), (String, &'static str)> {
    let Some(answered) = shared.installs.cancel(model) else {
        return Err((
            format!("no download of {model} is running or waiting"),
            NOT_DOWNLOADING,
        ));
    };
    log::info!("model.cancel: the download of {model} is cancelled");
    for install in answered {
        shared
            .events
            .emit(cancelled_event(&install.current, &install.next));
    }
    Ok(())
}

/// **Queries thread.** `model.remove`: deletes `model`'s files ([`crate::local::remove`]) and
/// answers with `models.listed`. Refused while a job, a call or an update holds it
/// (`model_in_use`).
pub fn remove(
    shared: &Shared,
    model: &str,
    reference: Option<&str>,
) -> Result<Value, (String, Option<&'static str>)> {
    let Some(row) = shared.registry.get(model) else {
        return Err((format!("{model} is not a registry model"), None));
    };
    match crate::local::remove(shared, row) {
        Ok(()) => {
            log::info!("model.remove: {model}'s files are deleted");
            Ok(crate::queries::catalogue(shared, &shared.models, reference))
        }
        Err(crate::local::RemoveError::InUse(why)) => Err((why, Some(MODEL_IN_USE))),
        Err(crate::local::RemoveError::Failed(why)) => Err((why, None)),
    }
}

/// Why a download cannot start for want of space: what must be free, and what is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoSpace {
    /// The download's remaining bytes plus [`SPACE_MARGIN`].
    pub needed: u64,
    /// The bytes free to this user where models go.
    pub free: u64,
}

/// **Worker.** Whether `row`'s download fits: its bytes not on disk yet plus [`SPACE_MARGIN`]
/// against the free space where models go. Nothing left to fetch always fits; free space the OS
/// cannot say does too (logged), so a failing query never blocks a download.
pub fn check_space(shared: &Shared, row: &EngineRow) -> Result<(), NoSpace> {
    let remaining = row
        .total_size()
        .saturating_sub(shared.models.bytes_on_disk(row));
    if remaining == 0 {
        return Ok(());
    }
    let Some(free) = free_bytes(shared) else {
        return Ok(());
    };
    let needed = remaining.saturating_add(SPACE_MARGIN);
    if free < needed {
        return Err(NoSpace { needed, free });
    }
    Ok(())
}

/// `command.failed`'s code: a download refused for want of space.
pub const NOT_ENOUGH_SPACE: &str = "not_enough_space";

/// `command.failed`'s code: a removal refused while the model is held.
pub const MODEL_IN_USE: &str = "model_in_use";

/// `command.failed`'s code: a cancel of a model no download is running or queued for.
pub const NOT_DOWNLOADING: &str = "not_downloading";
