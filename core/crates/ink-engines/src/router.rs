//! The router: job → the best engine installed for this OS (architecture rule 10).

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};

use ink_core::{EngineError, EngineInfo, Job, OfflineEngine, StreamingEngine};

use crate::model_dir::ModelDir;
use crate::registry::{EngineRow, JobScore, Os, Registry};

/// An engine the native shell registered (over the C ABI), already loaded and owned by the shell.
#[derive(Clone)]
pub enum ExternalEngine {
    /// Fills [`Job::DictationFinal`] and/or [`Job::MeetingFinal`].
    Offline(Arc<dyn OfflineEngine>),
    /// Fills [`Job::LivePartials`].
    Streaming(Arc<dyn StreamingEngine>),
}

impl ExternalEngine {
    /// What the engine says it is.
    pub fn info(&self) -> EngineInfo {
        match self {
            Self::Offline(e) => e.info(),
            Self::Streaming(e) => e.info(),
        }
    }
}

impl fmt::Debug for ExternalEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self {
            Self::Offline(_) => "Offline",
            Self::Streaming(_) => "Streaming",
        };
        f.debug_struct("ExternalEngine")
            .field("kind", &kind)
            .field("id", &self.info().id)
            .finish()
    }
}

/// The router's answer.
#[derive(Clone, Debug)]
pub enum Route {
    /// A registry model, installed on disk. Load it through
    /// [`Residency::acquire`](crate::Residency::acquire).
    Model(Arc<EngineRow>),
    /// An engine the shell registered. Call it directly.
    External(ExternalEngine),
}

impl Route {
    /// The chosen engine's id.
    pub fn id(&self) -> String {
        match self {
            Self::Model(row) => row.id.clone(),
            Self::External(e) => e.info().id,
        }
    }
}

/// Why routing or registration failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RouteError {
    /// Nothing installed for this OS fills the job. Never papered over with a default.
    NoEngine {
        /// The job.
        job: Job,
    },
    /// The id is taken by a registry row or another registered engine.
    AlreadyRegistered {
        /// The id.
        id: String,
    },
    /// The registration is malformed: no jobs, a job without a score or a score without a job, a
    /// job the engine's kind cannot fill, a non-finite error rate.
    Invalid {
        /// The engine.
        id: String,
        /// What is wrong.
        reason: String,
    },
}

impl fmt::Display for RouteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoEngine { job } => write!(f, "no engine installed for job {job:?}"),
            Self::AlreadyRegistered { id } => write!(f, "engine id {id} is already registered"),
            Self::Invalid { id, reason } => write!(f, "engine {id}: {reason}"),
        }
    }
}

impl std::error::Error for RouteError {}

impl From<RouteError> for EngineError {
    fn from(e: RouteError) -> Self {
        match e {
            RouteError::NoEngine { .. } => EngineError::ModelMissing(e.to_string()),
            other => EngineError::Failed(other.to_string()),
        }
    }
}

struct External {
    engine: ExternalEngine,
    scores: Vec<JobScore>,
}

/// Picks, for a job, the installed engine for this OS with the lowest measured error rate on that
/// job. Registry rows and engines the shell registered compete on the same terms.
///
/// Ties break deterministically: a shell-registered engine before a registry model (the shell
/// registers the engines that run on the platform's accelerators, architecture rule 2), then by id.
///
/// `Send + Sync`. Routing reads the disk (a size check per file) with no lock held; the lock over
/// registered engines is held only to copy them out or to insert or remove one.
pub struct Router {
    rows: Vec<Arc<EngineRow>>,
    dir: ModelDir,
    os: Os,
    externals: RwLock<BTreeMap<String, External>>,
}

impl Router {
    /// A router over `registry`'s rows installed in `dir`, for `os`.
    pub fn new(registry: &Registry, dir: ModelDir, os: Os) -> Self {
        Self {
            rows: registry.rows().iter().cloned().map(Arc::new).collect(),
            dir,
            os,
            externals: RwLock::default(),
        }
    }

    /// **Worker.** The best engine for `job`, or [`RouteError::NoEngine`].
    pub fn route(&self, job: Job) -> Result<Route, RouteError> {
        // Copied out so no lock is held while the disk is read.
        let externals: Vec<(f32, String, ExternalEngine)> = self
            .read()
            .iter()
            .filter_map(|(id, e)| {
                e.scores
                    .iter()
                    .find(|s| s.job == job)
                    .map(|s| (s.wer, id.clone(), e.engine.clone()))
            })
            .collect();

        // Sort key: error rate, then shell engines first, then id. Ids are unique across rows and
        // shell engines (registration refuses a clash), so the order is total.
        let mut best: Option<(f32, bool, &str, Route)> = None;
        let better =
            |wer: f32, is_model: bool, id: &str, best: &Option<(f32, bool, &str, Route)>| {
                best.as_ref().is_none_or(|(b_wer, b_model, b_id, _)| {
                    wer.total_cmp(b_wer)
                        .then(is_model.cmp(b_model))
                        .then(id.cmp(b_id))
                        .is_lt()
                })
            };
        for (wer, id, engine) in &externals {
            if better(*wer, false, id, &best) {
                best = Some((*wer, false, id, Route::External(engine.clone())));
            }
        }
        for row in &self.rows {
            let Some(wer) = row.wer(job) else { continue };
            if row.runs_on(self.os)
                && better(wer, true, &row.id, &best)
                && self.dir.is_installed(row)
            {
                best = Some((wer, true, &row.id, Route::Model(Arc::clone(row))));
            }
        }
        best.map(|(_, _, _, route)| route)
            .ok_or(RouteError::NoEngine { job })
    }

    /// **Worker.** Registers an offline engine from the shell, with its measured error rate for
    /// each job its [`EngineInfo`] lists. Offline engines may fill [`Job::DictationFinal`] and
    /// [`Job::MeetingFinal`].
    pub fn register_offline(
        &self,
        engine: Arc<dyn OfflineEngine>,
        scores: &[JobScore],
    ) -> Result<(), RouteError> {
        self.register(ExternalEngine::Offline(engine), scores)
    }

    /// **Worker.** Registers a streaming engine from the shell, like
    /// [`register_offline`](Self::register_offline). Streaming engines may fill
    /// [`Job::LivePartials`].
    pub fn register_streaming(
        &self,
        engine: Arc<dyn StreamingEngine>,
        scores: &[JobScore],
    ) -> Result<(), RouteError> {
        self.register(ExternalEngine::Streaming(engine), scores)
    }

    fn register(&self, engine: ExternalEngine, scores: &[JobScore]) -> Result<(), RouteError> {
        // Asked of the engine itself (with no lock held: it may be a call into the shell), so the
        // id and jobs it routes under are the ones it reports.
        let info = engine.info();
        check_registration(&info, &engine, scores)?;
        if self.rows.iter().any(|r| r.id == info.id) {
            return Err(RouteError::AlreadyRegistered { id: info.id });
        }
        let mut externals = self
            .externals
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        if externals.contains_key(&info.id) {
            return Err(RouteError::AlreadyRegistered { id: info.id });
        }
        externals.insert(
            info.id,
            External {
                engine,
                scores: scores.to_vec(),
            },
        );
        Ok(())
    }

    /// **Worker.** Removes a registered engine. Returns whether it was registered.
    pub fn unregister(&self, id: &str) -> bool {
        let removed = self
            .externals
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id);
        // Dropped here, after the lock: the last reference may release the shell's engine.
        removed.is_some()
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<String, External>> {
        self.externals
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// The jobs each kind of shell engine can fill. The shell registers no diarizer.
fn fills(engine: &ExternalEngine, job: Job) -> bool {
    match engine {
        ExternalEngine::Offline(_) => matches!(job, Job::DictationFinal | Job::MeetingFinal),
        ExternalEngine::Streaming(_) => matches!(job, Job::LivePartials),
    }
}

fn check_registration(
    info: &EngineInfo,
    engine: &ExternalEngine,
    scores: &[JobScore],
) -> Result<(), RouteError> {
    let invalid = |reason: String| RouteError::Invalid {
        id: info.id.clone(),
        reason,
    };
    if info.id.trim().is_empty() {
        return Err(invalid("empty id".into()));
    }
    if info.jobs.is_empty() {
        return Err(invalid("fills no job".into()));
    }
    for (i, &job) in info.jobs.iter().enumerate() {
        if info.jobs[..i].contains(&job) {
            return Err(invalid(format!("lists {job:?} twice")));
        }
        if !fills(engine, job) {
            return Err(invalid(format!("this kind of engine cannot fill {job:?}")));
        }
        if scores.iter().filter(|s| s.job == job).count() != 1 {
            return Err(invalid(format!("needs exactly one error rate for {job:?}")));
        }
    }
    for score in scores {
        if !info.jobs.contains(&score.job) {
            return Err(invalid(format!(
                "has an error rate for {:?}, which it does not list",
                score.job
            )));
        }
        if !score.wer.is_finite() || score.wer < 0.0 {
            return Err(invalid(format!(
                "error rate {} for {:?} is not a finite, non-negative number",
                score.wer, score.job
            )));
        }
    }
    Ok(())
}
