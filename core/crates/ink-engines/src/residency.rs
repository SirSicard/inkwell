//! Residency: which models are loaded, and when they are let go.
//!
//! - The dictation model stays warm ([`Residency::set_warm`]).
//! - Anything else loads when first asked for ([`Residency::acquire`]); the meeting-final model is
//!   therefore loaded at meeting end, when the final pass asks for it.
//! - A model nobody holds for [`IDLE_UNLOAD`] is unloaded by the next [`Residency::tick`].
//! - There is never more than one copy of a model, however many worker threads ask at once.
//! - [`Residency::unload`] drops a model now, when an update is about to replace its files.
//!
//! No timers: the pipeline calls `tick` when it wakes anyway, and the clock is injected, so tests
//! move time with a mock clock instead of sleeping.

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use ink_core::{Clock, EngineError};

use crate::lock;
use crate::registry::EngineRow;

/// How long a model may sit unused before [`Residency::tick`] unloads it.
pub const IDLE_UNLOAD: Duration = Duration::from_secs(5 * 60);

/// Loads a row's model. The adapters implement it; tests use a counting mock.
pub trait Loader<M>: Send + Sync {
    /// **Worker.** Loads `row`'s files. May take seconds. Unloading is dropping the returned
    /// value.
    fn load(&self, row: &EngineRow) -> Result<M, EngineError>;
}

/// [`IDLE_UNLOAD`] in the clock's unit.
const IDLE_NS: u64 = IDLE_UNLOAD.as_secs() * 1_000_000_000;

/// The loaded models. `Send + Sync`; see the module docs for the policy.
///
/// The lock guards bookkeeping only. It is never held while a model loads or unloads: a thread
/// that wants a model another thread is loading (or unloading) waits on a condition variable
/// instead, which is what keeps it to one copy without serialising unrelated models.
pub struct Residency<M> {
    loader: Arc<dyn Loader<M>>,
    shared: Arc<Shared<M>>,
}

struct Shared<M> {
    state: Mutex<State<M>>,
    /// Signalled whenever a slot leaves `Loading` or `Unloading`.
    changed: Condvar,
    clock: Arc<dyn Clock>,
}

struct State<M> {
    slots: HashMap<String, Slot<M>>,
    warm: Option<String>,
}

enum Slot<M> {
    /// One thread is loading it; others wait.
    Loading,
    Resident {
        /// Held by this slot and by every live lease. Whether a model is in use is read from this
        /// `Arc`'s strong count, not from a separate counter: a lease lets go of its clone only
        /// after its drop body has run, so a counter would read zero while the lease's clone still
        /// keeps the old copy alive, and a reload in that window would make two copies. Clones are
        /// only made under the lock, so a count of 1 seen under the lock cannot rise until it is
        /// released.
        model: Arc<M>,
        /// When the last lease was dropped (or the model stopped being warm).
        idle_since_ns: u64,
    },
    /// Being dropped outside the lock; a new load waits for it so two copies never overlap.
    Unloading,
}

impl<M> Shared<M> {
    fn lock(&self) -> MutexGuard<'_, State<M>> {
        lock(&self.state)
    }

    fn wait<'a>(&self, guard: MutexGuard<'a, State<M>>) -> MutexGuard<'a, State<M>> {
        self.changed
            .wait(guard)
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl<M: Send + Sync + 'static> Residency<M> {
    /// Loads through `loader`, measuring idle time on `clock`.
    pub fn new(loader: Arc<dyn Loader<M>>, clock: Arc<dyn Clock>) -> Self {
        Self {
            loader,
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    slots: HashMap::new(),
                    warm: None,
                }),
                changed: Condvar::new(),
                clock,
            }),
        }
    }

    /// **Worker.** The model for `row`, loading it if it is not resident. Blocks while another
    /// thread loads the same model rather than loading a second copy. If that load fails, each
    /// waiting thread tries again in turn and gets its own error.
    pub fn acquire(&self, row: &EngineRow) -> Result<Lease<M>, EngineError> {
        let id = row.id.as_str();
        let mut state = self.shared.lock();
        loop {
            match state.slots.get_mut(id) {
                Some(Slot::Resident { model, .. }) => {
                    return Ok(Lease {
                        model: Arc::clone(model),
                        id: id.to_string(),
                        shared: Arc::clone(&self.shared),
                    });
                }
                Some(Slot::Loading | Slot::Unloading) => state = self.shared.wait(state),
                None => break,
            }
        }
        state.slots.insert(id.to_string(), Slot::Loading);
        drop(state);

        // Clears the `Loading` slot if the load returns an error or panics, so waiters are not
        // left behind a load that will never finish.
        let mut pending = PendingLoad {
            shared: &self.shared,
            id,
            finished: false,
        };
        let model = Arc::new(self.loader.load(row)?);
        let now = self.shared.clock.now_ns();
        self.shared.lock().slots.insert(
            id.to_string(),
            Slot::Resident {
                model: Arc::clone(&model),
                idle_since_ns: now,
            },
        );
        pending.finished = true;
        self.shared.changed.notify_all();
        Ok(Lease {
            model,
            id: id.to_string(),
            shared: Arc::clone(&self.shared),
        })
    }

    /// **Worker.** Keeps `row`'s model loaded regardless of idle time, loading it now if needed,
    /// or with `None` stops keeping any model warm. The model that was warm before starts its idle
    /// time now.
    pub fn set_warm(&self, row: Option<&EngineRow>) -> Result<(), EngineError> {
        // Loaded first, and held until it is marked warm so a tick in between cannot unload it.
        let lease = row.map(|r| self.acquire(r)).transpose()?;
        let now = self.shared.clock.now_ns();
        let mut state = self.shared.lock();
        let previous = std::mem::replace(&mut state.warm, row.map(|r| r.id.clone()));
        if let Some(previous) = previous.filter(|p| state.warm.as_ref() != Some(p))
            && let Some(Slot::Resident { idle_since_ns, .. }) = state.slots.get_mut(&previous)
        {
            *idle_since_ns = now;
        }
        drop(state);
        // After the lock: dropping a lease takes it.
        drop(lease);
        Ok(())
    }

    /// The id of the model kept warm.
    pub fn warm(&self) -> Option<String> {
        self.shared.lock().warm.clone()
    }

    /// **Worker.** Unloads every model that is not warm, not held, and idle for at least
    /// [`IDLE_UNLOAD`]. Returns their ids, sorted. The models are dropped with no lock held.
    pub fn tick(&self) -> Vec<String> {
        let now = self.shared.clock.now_ns();
        let mut state = self.shared.lock();
        let State { slots, warm } = &mut *state;
        let mut due: Vec<String> = slots
            .iter()
            .filter(|(id, slot)| {
                matches!(slot, Slot::Resident { model, idle_since_ns }
                    if Arc::strong_count(model) == 1
                        && now.saturating_sub(*idle_since_ns) >= IDLE_NS)
                    && warm.as_ref() != Some(*id)
            })
            .map(|(id, _)| id.clone())
            .collect();
        if due.is_empty() {
            return due;
        }
        due.sort();
        let mut models = Vec::with_capacity(due.len());
        for id in &due {
            if let Some(slot) = slots.get_mut(id)
                && let Slot::Resident { model, .. } = std::mem::replace(slot, Slot::Unloading)
            {
                models.push(model);
            }
        }
        drop(state);
        {
            // Created after the lock is released (its drop takes the lock), and alive while the
            // models drop, so the `Unloading` slots are cleared even if a destructor panics.
            let _unloading = Unloading {
                shared: &self.shared,
                ids: &due,
            };
            drop(models);
        }
        due
    }

    /// **Worker.** Unloads the model `id` now, for an update that replaces its files: it stops
    /// being kept warm and is dropped, with no lock held (its drop may take seconds, or call back
    /// into residency). Waits out a load or unload of the same model already under way.
    ///
    /// Refused while any lease holds the model, and then nothing changes (it stays loaded, and
    /// warm if it was): the update waits for the job to end and tries again. An id that is not
    /// loaded is already unloaded, so that is `Ok`.
    ///
    /// Unloading does not keep a model out: a later [`acquire`](Self::acquire) or
    /// [`set_warm`](Self::set_warm) loads it again, so the caller keeps it out of use until the
    /// new files are in place.
    pub fn unload(&self, id: &str) -> Result<(), EngineError> {
        let mut state = self.shared.lock();
        while matches!(state.slots.get(id), Some(Slot::Loading | Slot::Unloading)) {
            state = self.shared.wait(state);
        }
        let State { slots, warm } = &mut *state;
        let Some(slot) = slots.get_mut(id) else {
            return Ok(());
        };
        if matches!(slot, Slot::Resident { model, .. } if Arc::strong_count(model) > 1) {
            return Err(EngineError::Failed(format!(
                "model {id} is in use; unload it once its current job has finished"
            )));
        }
        let model = match std::mem::replace(slot, Slot::Unloading) {
            Slot::Resident { model, .. } => model,
            // Not reachable: the lock has been held since the loop saw neither of these. Put it
            // back rather than leave a slot that nothing will ever clear.
            other @ (Slot::Loading | Slot::Unloading) => {
                *slot = other;
                return Ok(());
            }
        };
        if warm.as_deref() == Some(id) {
            *warm = None;
        }
        drop(state);
        let ids = [id.to_string()];
        {
            // As in `tick`: clears the `Unloading` slot and wakes waiters even if the drop panics.
            let _unloading = Unloading {
                shared: &self.shared,
                ids: &ids,
            };
            drop(model);
        }
        Ok(())
    }

    /// The ids of the loaded models, sorted.
    pub fn resident(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .shared
            .lock()
            .slots
            .iter()
            .filter(|(_, slot)| matches!(slot, Slot::Resident { .. }))
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        ids
    }
}

struct PendingLoad<'a, M> {
    shared: &'a Shared<M>,
    id: &'a str,
    finished: bool,
}

impl<M> Drop for PendingLoad<'_, M> {
    fn drop(&mut self) {
        if !self.finished {
            let mut state = self.shared.lock();
            if matches!(state.slots.get(self.id), Some(Slot::Loading)) {
                state.slots.remove(self.id);
            }
            drop(state);
            self.shared.changed.notify_all();
        }
    }
}

struct Unloading<'a, M> {
    shared: &'a Shared<M>,
    ids: &'a [String],
}

impl<M> Drop for Unloading<'_, M> {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        for id in self.ids {
            if matches!(state.slots.get(id), Some(Slot::Unloading)) {
                state.slots.remove(id);
            }
        }
        drop(state);
        self.shared.changed.notify_all();
    }
}

/// A held model. While any lease on a model exists, it is not unloaded; its idle time starts when
/// the last lease is dropped. Leases do not hand out the model's `Arc`, so no copy can outlive
/// residency's bookkeeping.
pub struct Lease<M> {
    model: Arc<M>,
    id: String,
    shared: Arc<Shared<M>>,
}

impl<M> Lease<M> {
    /// The model's registry id.
    pub fn id(&self) -> &str {
        &self.id
    }
}

impl<M> Deref for Lease<M> {
    type Target = M;

    fn deref(&self) -> &M {
        &self.model
    }
}

impl<M> Drop for Lease<M> {
    fn drop(&mut self) {
        let now = self.shared.clock.now_ns();
        if let Some(Slot::Resident { idle_since_ns, .. }) =
            self.shared.lock().slots.get_mut(&self.id)
        {
            *idle_since_ns = now;
        }
        // `self.model` drops after this body, outside the lock; see `Slot::Resident`.
    }
}
