//! The core's own language model on this machine: a registry language row (Windows' Qwen3,
//! [`RowKind::Language`]) the core downloads, loads and calls itself through llama.cpp, for
//! polish, voice edit, a meeting's summary and Ask. The Mac has no such row: Apple's on-device
//! model, which the shell registers, does that there.
//!
//! - **One installed at a time.** The model in use is the installed language row the core last
//!   finished installing ([`CURRENT_KEY`]); installing the other size replaces it once its
//!   download has verified ([`installed`]), so there is never a moment with none.
//! - **Its own residency**, apart from the speech models', so it never evicts Qwen3-ASR. Loaded on
//!   first use, or warmed when a take starts that will use it ([`LocalLlms::take_started`]), and
//!   unloaded after [`IDLE_UNLOAD`](ink_engines::IDLE_UNLOAD) unused. No timer ticks: one thread,
//!   `ink-llm-local`, sleeps until the next unload is due, or until something happens.
//! - **Behind the model gate.** Every call registers its use ([`ModelGate::enter`]) before it
//!   loads, so an update or a removal ([`remove`]) never replaces or deletes files under a call,
//!   and a call never loads files being replaced.
//! - **One alias.** It is `engine:local` ([`LOCAL_ID`]) whichever size is installed, so a mode
//!   pinned to it follows a size change, and two modes never load two sizes.
//! - **On this machine:** its endpoint is [`Endpoint::InProcess`], so local-only mode lets it
//!   through, and the on-device consent covers it. A feature still needs that consent: the core
//!   fails closed without it.

use std::io;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ink_core::{
    CancelToken, Clock, Endpoint, EngineError, Llm, LlmError, LlmInfo, LlmRequest, LlmResponse,
};
use ink_engines::{EngineRow, Loader, Os, Residency, RowKind, Unloaded};
use ink_pipeline::consent::Feature;

use crate::gate::{ModelGate, Refused};
use crate::llms::ModelRef;
use crate::runtime::{Shared, lock};

/// What the local model's loader gives: a language model loaded in this process.
pub type LocalModel = Box<dyn Llm>;

/// The local model's id among the language models the core holds: `engine:local` in a mode's
/// `polish_model`, whichever size is installed.
pub const LOCAL_ID: &str = "local";

/// The context the local model is given, in tokens: the same as Apple's on-device model, so a
/// long meeting's summary is written in the windows already proven on the Mac.
pub const LOCAL_CONTEXT_TOKENS: u32 = 4_096;

/// The core's setting naming the language row last installed, the one in use while it is
/// installed. Not a shell setting.
pub const CURRENT_KEY: &str = "llm.local.model";

/// The local model's residency and the gate its calls go through. `Send + Sync`.
pub struct LocalLlms {
    residency: Residency<LocalModel>,
    gate: Arc<ModelGate>,
    clock: Arc<dyn Clock>,
    /// The `ink-llm-local` thread's mailbox, once it runs.
    mailbox: Mutex<Option<Sender<Wake>>>,
}

/// What wakes `ink-llm-local`.
enum Wake {
    /// A take started; warm the model if this take will use it.
    Take {
        edit: bool,
        mode: Option<String>,
    },
    /// A call let go of the model: when the next unload is due may have changed.
    Used,
    Quit,
}

impl LocalLlms {
    /// Loads through `loader`, measuring idle time on `clock`; calls go through `gate`.
    pub fn new(
        loader: Arc<dyn Loader<LocalModel>>,
        clock: Arc<dyn Clock>,
        gate: Arc<ModelGate>,
    ) -> Self {
        Self {
            residency: Residency::new(loader, clock.clone()),
            gate,
            clock,
            mailbox: Mutex::new(None),
        }
    }

    /// The model `row` installs, to call: `engine:local` while it is the one in use.
    pub fn handle(self: &Arc<Self>, row: &EngineRow) -> Option<Arc<LocalLlm>> {
        let RowKind::Language(language) = &row.kind else {
            return None;
        };
        Some(Arc::new(LocalLlm {
            row: row.clone(),
            info: LlmInfo {
                provider: LOCAL_ID.into(),
                model: language.name.clone(),
                endpoint: Endpoint::InProcess,
            },
            owner: self.clone(),
        }))
    }

    /// The ids loaded now, sorted.
    pub fn resident(&self) -> Vec<String> {
        self.residency.resident()
    }

    /// **Worker.** Unloads what has been idle for [`IDLE_UNLOAD`](ink_engines::IDLE_UNLOAD);
    /// the ids unloaded. `ink-llm-local` calls it when the next unload is due.
    pub fn tick(&self) -> Vec<String> {
        self.residency.tick()
    }

    /// **Worker.** Unloads `id` now, before its files are replaced or deleted. Refused while a
    /// call holds it.
    pub fn unload(&self, id: &str) -> Result<Unloaded, EngineError> {
        self.residency.unload(id)
    }

    /// **Any thread.** A take started (`edit`: a voice edit; `mode`: the mode a dictation writes
    /// in, by name). The model is warmed off this thread if the take will use it, so the take's
    /// own seconds hide its load.
    pub fn take_started(&self, edit: bool, mode: Option<String>) {
        self.send(Wake::Take { edit, mode });
    }

    fn send(&self, wake: Wake) {
        if let Some(tx) = lock(&self.mailbox).as_ref() {
            // A stopped thread drops it: the core is shutting down.
            let _ = tx.send(wake);
        }
    }

    /// The model `row` as a call holds it: through the gate (refused while an update or a
    /// removal holds it), then from residency, loading it if it is not loaded.
    fn enter(&self, row: &EngineRow) -> Result<Held<'_>, LlmError> {
        let used = self.gate.enter(&row.id).map_err(|refused| match refused {
            Refused::Held(_) => {
                LlmError::Engine("the on-device model is being updated or removed".into())
            }
            Refused::InUse(_) => LlmError::Engine("the on-device model is busy".into()),
        })?;
        let lease = self
            .residency
            .acquire(row)
            .map_err(|e| LlmError::Engine(format!("the on-device model did not load: {e}")))?;
        Ok(Held {
            lease,
            _used: used,
            _then: Told(self),
        })
    }

    /// **Worker.** Loads `row`'s model if it is not loaded, and lets go of it: it stays loaded
    /// until it has been idle for [`IDLE_UNLOAD`](ink_engines::IDLE_UNLOAD).
    fn warm(&self, row: &EngineRow) {
        match self.enter(row) {
            Ok(held) => drop(held),
            Err(e) => log::warn!("warming the on-device model failed: {e}"),
        }
    }
}

/// A call's hold on the model: its lease, then its gate use, let go of in that order (fields drop
/// in declaration order), and the warmer told last, when the model's idle time has started.
struct Held<'a> {
    lease: ink_engines::Lease<LocalModel>,
    _used: crate::gate::Use,
    _then: Told<'a>,
}

impl Held<'_> {
    fn model(&self) -> &dyn Llm {
        &**self.lease
    }
}

/// Tells `ink-llm-local` that a call let go of the model, when dropped.
struct Told<'a>(&'a LocalLlms);

impl Drop for Told<'_> {
    fn drop(&mut self) {
        self.0.send(Wake::Used);
    }
}

/// The local model, as the features call it ([`Llm`]). Its info names the row's model and says
/// it is in this process.
pub struct LocalLlm {
    row: EngineRow,
    info: LlmInfo,
    owner: Arc<LocalLlms>,
}

impl LocalLlm {
    /// The registry row it is.
    pub fn row(&self) -> &EngineRow {
        &self.row
    }

    /// **Worker.** Loads the model if it is not loaded, then sends `request`, timing each: the
    /// milliseconds the load took (near zero when it was loaded already) and the answer took
    /// (`llm.test`'s timing).
    pub fn timed(
        &self,
        request: &LlmRequest,
        cancel: &CancelToken,
    ) -> (u64, Option<u64>, Result<LlmResponse, LlmError>) {
        let clock = &self.owner.clock;
        let started = clock.now_ns();
        let held = match self.owner.enter(&self.row) {
            Ok(held) => held,
            Err(e) => return (ms(clock.now_ns() - started), None, Err(e)),
        };
        let loaded = clock.now_ns();
        let answer = held.model().complete(request, cancel);
        let answered = clock.now_ns();
        drop(held);
        (
            ms(loaded.saturating_sub(started)),
            Some(ms(answered.saturating_sub(loaded))),
            answer,
        )
    }
}

fn ms(ns: u64) -> u64 {
    ns / 1_000_000
}

impl Llm for LocalLlm {
    fn info(&self) -> LlmInfo {
        self.info.clone()
    }

    fn complete(
        &self,
        request: &LlmRequest,
        cancel: &CancelToken,
    ) -> Result<LlmResponse, LlmError> {
        if cancel.is_cancelled() {
            return Err(LlmError::Cancelled);
        }
        let held = self.owner.enter(&self.row)?;
        held.model().complete(request, cancel)
    }
}

/// `ink-llm-local`: warms the model when a take that uses it starts, and unloads it once it has
/// been idle long enough. It sleeps until the next unload is due, or a message comes.
pub struct LocalThread {
    tx: Sender<Wake>,
    thread: JoinHandle<()>,
}

impl LocalThread {
    /// Starts it and gives [`Shared::local`] its mailbox.
    pub fn start(shared: Arc<Shared>) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel::<Wake>();
        let thread = {
            let shared = shared.clone();
            thread::Builder::new()
                .name("ink-llm-local".into())
                .spawn(move || run(&shared, &rx))?
        };
        *lock(&shared.local.mailbox) = Some(tx.clone());
        Ok(Self { tx, thread })
    }

    /// Ends the thread once a warm-up under way is done.
    pub fn stop(self) {
        let _ = self.tx.send(Wake::Quit);
        if self.thread.join().is_err() {
            log::error!("the on-device model's thread panicked outside its boundary");
        }
    }
}

fn run(shared: &Shared, rx: &Receiver<Wake>) {
    let local = &shared.local;
    loop {
        let wake = match local.residency.next_unload_ns() {
            Some(due) => {
                let wait = Duration::from_nanos(due.saturating_sub(local.clock.now_ns()));
                rx.recv_timeout(wait)
            }
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match wake {
            Ok(Wake::Take { edit, mode }) => {
                let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if let Some(row) = to_warm(shared, edit, mode.as_deref()) {
                        local.warm(&row);
                    }
                }));
                if ran.is_err() {
                    log::error!("warming the on-device model panicked; the thread goes on");
                }
            }
            Ok(Wake::Used) => {}
            Err(RecvTimeoutError::Timeout) => {
                let unloaded = local.tick();
                if !unloaded.is_empty() {
                    log::info!("on-device model unloaded after idling: {unloaded:?}");
                }
            }
            Ok(Wake::Quit) | Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// **Worker.** The local model's row, if the take that just started will use it: an edit goes to
/// the AI setting's model; a dictation is polished (the polish switch is on, its mode polishes)
/// on its mode's own model, else on the AI setting's. Either only with the user's consent for
/// this machine: without it the call is refused, and loading gigabytes for it would be waste.
fn to_warm(shared: &Shared, edit: bool, mode: Option<&str>) -> Option<EngineRow> {
    let local = shared.llms.local()?;
    let feature = if edit { Feature::Edit } else { Feature::Polish };
    let info = local.info();
    if !ink_pipeline::consent::stored(shared.store.as_ref(), feature)
        .iter()
        .any(|c| c.covers(&info))
    {
        return None;
    }
    let setting_is_local = shared.llms.pick_ref() == Some(ModelRef::Engine(LOCAL_ID.into()));
    if edit {
        return setting_is_local.then(|| local.row().clone());
    }
    let polish_on = shared
        .store
        .setting(crate::voice::POLISH_SETTING)
        .is_ok_and(|v| v.as_deref() == Some("on"));
    if !polish_on {
        return None;
    }
    // Modes that cannot be read polish nothing, so there is nothing to warm.
    let modes = crate::modes::load(shared.store.as_ref()).ok()?;
    let mode = match mode {
        Some(name) => modes.find_by_name(name),
        None => modes.modes.iter().find(|m| m.id == modes.default_id),
    }?;
    if !mode.polish_enabled {
        return None;
    }
    let uses_local = match &mode.polish_model {
        Some(pin) => ModelRef::parse(&pin.id) == Some(ModelRef::Engine(LOCAL_ID.into())),
        None => setting_is_local,
    };
    uses_local.then(|| local.row().clone())
}

/// **Worker.** Finds the language row in use (the one [`CURRENT_KEY`] names while it is
/// installed, else the first installed in the registry's order) and gives it to the features as
/// `engine:local`, or none. Called at launch and after an install or a removal.
pub fn refresh(shared: &Shared) {
    let current = current_row(shared);
    shared
        .llms
        .set_local(current.and_then(|row| shared.local.handle(row)));
}

/// The installed language rows for this OS, in the registry's order.
fn installed_rows(shared: &Shared) -> Vec<&EngineRow> {
    let Some(os) = Os::current() else {
        return Vec::new();
    };
    shared
        .registry
        .rows()
        .iter()
        .filter(|r| r.runs_on(os) && crate::models::is_language(r) && shared.models.is_installed(r))
        .collect()
}

fn current_row(shared: &Shared) -> Option<&EngineRow> {
    let installed = installed_rows(shared);
    let named = match shared.store.setting(CURRENT_KEY) {
        Ok(v) => v,
        Err(e) => {
            log::warn!(
                "the on-device model in use could not be read ({e}); the first installed is used"
            );
            None
        }
    };
    named
        .and_then(|id| installed.iter().copied().find(|r| r.id == id))
        .or_else(|| installed.first().copied())
}

/// **Worker.** At launch: one language model installed at a time. One that an earlier
/// replacement could not delete (a call held it then) is deleted now, before anything can load
/// it. Only while [`CURRENT_KEY`] names an installed row: without it, which one the user wants
/// is not known, and nothing is deleted.
pub fn tidy(shared: &Shared) {
    let Ok(Some(current)) = shared.store.setting(CURRENT_KEY) else {
        return;
    };
    let installed = installed_rows(shared);
    if !installed.iter().any(|r| r.id == current) {
        return;
    }
    for row in installed.into_iter().filter(|r| r.id != current) {
        if let Err(e) = remove(shared, row) {
            log::warn!(
                "the on-device model {} replaced earlier was not deleted: {e}",
                row.id
            );
        }
    }
}

/// **Worker.** After `row`, a language row, has installed and verified: it is the one in use from
/// now on, and the other language rows are deleted (one installed at a time). A row a call holds
/// is left, and deleted at the next launch ([`tidy`]). A failure to record which one is in use is
/// logged; the new one is still used while it is the only one installed.
pub fn installed(shared: &Shared, row: &EngineRow) {
    if let Err(e) = shared.store.set_setting(CURRENT_KEY, &row.id) {
        log::error!("the on-device model in use could not be saved ({e})");
    }
    refresh(shared);
    let others: Vec<&EngineRow> = installed_rows(shared)
        .into_iter()
        .filter(|r| r.id != row.id)
        .collect();
    for other in others {
        match remove(shared, other) {
            Ok(()) => log::info!("on-device model {} replaced by {}", other.id, row.id),
            Err(e) => log::warn!(
                "on-device model {} replaced by {}, but not deleted yet ({e}); the next launch deletes it",
                other.id,
                row.id
            ),
        }
    }
    refresh(shared);
}

/// Why a model's files were not deleted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoveError {
    /// A job or a call holds it, or an update does: nothing was deleted.
    InUse(String),
    /// Deleting failed (the message names the step, never a path's contents).
    Failed(String),
}

impl std::fmt::Display for RemoveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InUse(why) | Self::Failed(why) => f.write_str(why),
        }
    }
}

/// **Worker.** Deletes `row`'s files, speech or language: refused while a job, a call or an update
/// holds it ([`ModelGate::hold`]); otherwise it is held for the delete, unloaded from whichever
/// residency has it, and its directory is deleted ([`ModelDir::remove_row`](ink_engines::ModelDir::remove_row),
/// which stays inside the models root). A removed language model in use stops being used (the
/// features then have none of their own; nothing falls through to a cloud provider).
pub fn remove(shared: &Shared, row: &EngineRow) -> Result<(), RemoveError> {
    let hold = shared
        .gate
        .hold(&[row.id.as_str()])
        .map_err(|refused| RemoveError::InUse(refused.to_string()))?;
    let language = crate::models::is_language(row);
    let unloaded = if language {
        shared.local.unload(&row.id)
    } else {
        shared.residency.unload(&row.id)
    };
    if let Err(e) = unloaded {
        return Err(RemoveError::InUse(e.to_string()));
    }
    let removed = shared.models.remove_row(row).map_err(|e| {
        log::warn!(
            "model {}: its files could not be deleted ({})",
            row.id,
            e.kind()
        );
        RemoveError::Failed(format!("couldn't delete {}'s files: {e}", row.id))
    });
    drop(hold);
    if language {
        refresh(shared);
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use ink_core::mock::MockClock;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Answering {
        loads: AtomicUsize,
    }

    struct Model;

    impl Llm for Model {
        fn info(&self) -> LlmInfo {
            LlmInfo {
                provider: "test".into(),
                model: "inner".into(),
                endpoint: Endpoint::InProcess,
            }
        }

        fn complete(&self, _: &LlmRequest, _: &CancelToken) -> Result<LlmResponse, LlmError> {
            Ok(LlmResponse { text: "ok".into() })
        }
    }

    impl Loader<LocalModel> for Answering {
        fn load(&self, _: &EngineRow) -> Result<LocalModel, EngineError> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Model))
        }
    }

    fn row() -> EngineRow {
        let mut row = ink_engines::qwen3_1_7b_q8();
        row.oses = vec![Os::MacOs, Os::Windows];
        row
    }

    fn request() -> LlmRequest {
        LlmRequest {
            system: String::new(),
            user: "synthetic".into(),
            max_tokens: 8,
            temperature: 0.0,
            json_schema: None,
        }
    }

    #[test]
    fn a_call_loads_once_and_the_model_unloads_after_five_idle_minutes() {
        let clock = Arc::new(MockClock::new(1_000, 0));
        let loader = Arc::new(Answering {
            loads: AtomicUsize::new(0),
        });
        let local = Arc::new(LocalLlms::new(
            loader.clone(),
            clock.clone(),
            Arc::default(),
        ));
        let llm = local.handle(&row()).unwrap();
        assert_eq!(llm.info().endpoint, Endpoint::InProcess);
        assert_eq!(llm.info().model, "Qwen3 1.7B");
        for _ in 0..3 {
            let answer = llm.complete(&request(), &CancelToken::new()).unwrap();
            assert_eq!(answer.text, "ok");
        }
        assert_eq!(loader.loads.load(Ordering::SeqCst), 1, "loaded once");
        assert_eq!(local.resident(), [row().id]);
        clock.advance_ns(u64::try_from(ink_engines::IDLE_UNLOAD.as_nanos()).unwrap() - 1);
        assert!(local.tick().is_empty());
        clock.advance_ns(1);
        assert_eq!(local.tick(), [row().id]);
        assert!(local.resident().is_empty());
    }

    #[test]
    fn a_call_is_refused_while_an_update_or_removal_holds_the_model() {
        let clock = Arc::new(MockClock::new(1_000, 0));
        let loader = Arc::new(Answering {
            loads: AtomicUsize::new(0),
        });
        let gate: Arc<ModelGate> = Arc::default();
        let local = Arc::new(LocalLlms::new(loader.clone(), clock, gate.clone()));
        let llm = local.handle(&row()).unwrap();
        let hold = gate.hold(&[row().id.as_str()]).unwrap();
        let refused = llm.complete(&request(), &CancelToken::new()).unwrap_err();
        assert!(matches!(refused, LlmError::Engine(_)), "{refused}");
        assert_eq!(loader.loads.load(Ordering::SeqCst), 0, "nothing loaded");
        drop(hold);
        llm.complete(&request(), &CancelToken::new()).unwrap();
    }

    #[test]
    fn a_speech_row_is_no_language_model() {
        let local = Arc::new(LocalLlms::new(
            Arc::new(Answering {
                loads: AtomicUsize::new(0),
            }),
            Arc::new(MockClock::new(1_000, 0)),
            Arc::default(),
        ));
        assert!(local.handle(&ink_engines::silero_vad()).is_none());
    }
}
