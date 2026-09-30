//! Exclusive holds on models during an update, and the engine every chain calls through.
//!
//! [`Residency::unload`] drops a model before an update replaces its files, but nothing in
//! residency keeps it out afterwards: the next [`Residency::acquire`] loads it again, possibly
//! from files half replaced. The [`ModelGate`] closes that window. An update takes a [`Hold`] on
//! the model's id (and the new one's) before it unloads anything, and keeps it until the new
//! model is installed and warm. While it is held, every job that needs the model is refused,
//! with a `model.refused` event, before residency is asked for anything.
//!
//! The other direction is covered too: a job registers its use ([`ModelGate::enter`]) before it
//! acquires, so an update cannot take a hold between a job's check and its load. A hold is
//! refused while any job is inside, as residency refuses to unload a leased model: the update
//! reports it and can be retried when the job is done.
//!
//! [`Routed`] is the [`OfflineEngine`] a chain is given for its job. Each call routes the job
//! afresh (so an engine registered or updated since is used), goes through the gate for a
//! registry model, and calls a shell engine directly.
//!
//! [`Residency::unload`]: ink_engines::Residency::unload
//! [`Residency::acquire`]: ink_engines::Residency::acquire

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ink_core::{
    EngineError, EngineInfo, Job, OfflineEngine, StreamingEngine, TranscribeOptions, Transcript,
};
use ink_engines::{EngineRow, ExternalEngine, Route, TrailingWindow};
use serde_json::Value;

use crate::events::{self, event};
use crate::runtime::Shared;

#[derive(Default)]
struct State {
    held: HashSet<String>,
    users: HashMap<String, usize>,
}

/// See the module docs. `Send + Sync`; its lock guards two small maps and is never held across
/// a load, an install or an engine call.
#[derive(Default)]
pub struct ModelGate {
    state: Mutex<State>,
}

/// Why a hold or an entry was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The model is held by an update.
    Held(String),
    /// A job is using the model.
    InUse(String),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Held(id) => write!(f, "model {id} is being updated"),
            Self::InUse(id) => write!(f, "model {id} is in use; update it once its job is done"),
        }
    }
}

impl ModelGate {
    fn lock(&self) -> MutexGuard<'_, State> {
        // Each critical section is a few set and map operations: never inconsistent mid-way.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// **Worker.** Holds every id in `ids` exclusively, or none of them: refused when one is
    /// already held, or in use.
    pub fn hold(self: &Arc<Self>, ids: &[&str]) -> Result<Hold, Refused> {
        let mut s = self.lock();
        for id in ids {
            if s.held.contains(*id) {
                return Err(Refused::Held((*id).to_owned()));
            }
            if s.users.get(*id).is_some_and(|n| *n > 0) {
                return Err(Refused::InUse((*id).to_owned()));
            }
        }
        let ids: Vec<String> = ids.iter().map(|id| (*id).to_owned()).collect();
        s.held.extend(ids.iter().cloned());
        Ok(Hold {
            gate: self.clone(),
            ids,
        })
    }

    /// **Worker.** Registers a use of `id`, unless it is held.
    pub fn enter(self: &Arc<Self>, id: &str) -> Result<Use, Refused> {
        let mut s = self.lock();
        if s.held.contains(id) {
            return Err(Refused::Held(id.to_owned()));
        }
        *s.users.entry(id.to_owned()).or_default() += 1;
        Ok(Use {
            gate: self.clone(),
            id: id.to_owned(),
        })
    }

    /// Whether `id` is held now.
    pub fn is_held(&self, id: &str) -> bool {
        self.lock().held.contains(id)
    }
}

/// An update's exclusive hold. Released on drop, however the update ends.
pub struct Hold {
    gate: Arc<ModelGate>,
    ids: Vec<String>,
}

impl Drop for Hold {
    fn drop(&mut self) {
        let mut s = self.gate.lock();
        for id in &self.ids {
            s.held.remove(id);
        }
    }
}

/// A job's use of a model. Ends on drop.
pub struct Use {
    gate: Arc<ModelGate>,
    id: String,
}

impl Drop for Use {
    fn drop(&mut self) {
        let mut s = self.gate.lock();
        if let Some(n) = s.users.get_mut(&self.id) {
            *n -= 1;
            if *n == 0 {
                s.users.remove(&self.id);
            }
        }
    }
}

/// `model.refused`.
pub fn refused_event(id: &str, job: Job) -> Value {
    event(
        "model.refused",
        &[
            ("id", Some(id.into())),
            ("job", Some(events::job(job).into())),
            ("reason", Some("updating".into())),
        ],
    )
}

/// The engine a chain is given for `job`. See the module docs.
pub struct Routed {
    shared: Arc<Shared>,
    job: Job,
}

impl Routed {
    /// The engine for `job`.
    pub fn new(shared: Arc<Shared>, job: Job) -> Self {
        Self { shared, job }
    }
}

impl OfflineEngine for Routed {
    fn info(&self) -> EngineInfo {
        match self.shared.router.route(self.job) {
            Ok(Route::Model(row)) => row.info(),
            Ok(Route::External { engine, .. }) => engine.info(),
            Err(_) => EngineInfo {
                id: "none".into(),
                jobs: vec![self.job],
                licence: String::new(),
            },
        }
    }

    fn transcribe(
        &self,
        audio: &[f32],
        options: &TranscribeOptions,
    ) -> Result<Transcript, EngineError> {
        match self.shared.router.route(self.job)? {
            Route::External {
                engine: ExternalEngine::Offline(engine),
                ..
            } => engine.transcribe(audio, options),
            Route::External { .. } => Err(EngineError::Unsupported(
                "a streaming engine was routed an offline job",
            )),
            Route::Model(row) => {
                let _use = self.shared.gate.enter(&row.id).map_err(|refused| {
                    log::info!("{refused}; the {:?} job is refused", self.job);
                    self.shared.events.emit(refused_event(&row.id, self.job));
                    EngineError::Failed(format!(
                        "{refused}; try again when the update has finished"
                    ))
                })?;
                let lease = self.shared.residency.acquire(&row)?;
                lease.transcribe(audio, options)
            }
        }
    }
}

/// Live partials from a registry model the core runs itself (Windows' Parakeet): the trailing
/// window scheme ([`TrailingWindow`]) over [`Routed`] for [`Job::LivePartials`], so each window's
/// decode goes through the gate and residency like any other job, and the model is the one copy
/// dictation uses too. Reported as `row`.
pub fn live_model(shared: &Arc<Shared>, row: &EngineRow) -> Arc<dyn StreamingEngine> {
    Arc::new(TrailingWindow::new(
        Arc::new(Routed::new(shared.clone(), Job::LivePartials)),
        row.info(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hold_refuses_entry_and_is_refused_while_in_use() {
        let gate = Arc::new(ModelGate::default());
        let used = gate.enter("m").unwrap();
        assert_eq!(gate.hold(&["m"]).err(), Some(Refused::InUse("m".into())));
        drop(used);
        let hold = gate.hold(&["m", "n"]).unwrap();
        assert_eq!(gate.enter("n").err(), Some(Refused::Held("n".into())));
        assert_eq!(gate.hold(&["m"]).err(), Some(Refused::Held("m".into())));
        assert!(gate.enter("other").is_ok());
        drop(hold);
        assert!(!gate.is_held("m"));
        assert!(gate.enter("m").is_ok());
    }

    #[test]
    fn a_refused_hold_takes_nothing() {
        let gate = Arc::new(ModelGate::default());
        let _used = gate.enter("b").unwrap();
        assert!(gate.hold(&["a", "b"]).is_err());
        assert!(!gate.is_held("a"), "all or nothing");
    }
}
