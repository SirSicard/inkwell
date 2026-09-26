//! The thread that owns a dictation chain.
//!
//! Everything reaches the chain through one queue, in order: audio from the pump, hotkey events
//! from the platform's callback thread, and settings from the shell. The hotkey callback only
//! enqueues (the OS disables an event tap that is slow), and the pump never waits on the chain,
//! whose engine call can take a second. The queue is unbounded: a dictation's audio is seconds.
//!
//! The thread blocks on the queue; it has no timer. The one deadline, a tail whose audio stopped
//! arriving, becomes the timeout of that wait.
//!
//! # Panics
//!
//! Each input runs behind a panic boundary. A stage that panics (an engine, the store, polish,
//! the inserter, or a bug here) costs the take in progress, never the worker: the chain is put
//! back to idle, the shell gets [`DictationEvent::WorkerFailed`] with `recovered: true`, and the
//! next take runs. If [`MAX_PANICS_WITHOUT_A_TAKE`] panics come with no take completing between
//! them, recovery is not working: the worker reports `recovered: false` and stops, rather than
//! fail on every key press forever. An input sent after that is not dropped silently:
//! [`WorkerHealth`] counts it and says the worker is gone, and [`DictationWorker::send`] returns
//! [`WorkerGone`].
//!
//! [`DictationEvent::WorkerFailed`]: crate::events::DictationEvent::WorkerFailed

use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ink_core::{Clock, EventSink, HotkeyEvent};

use crate::chain::{DictationChain, DictationSettings};
use crate::gain_stage::Vad;

/// Panics in a row, with no take completing between them, after which the worker stops.
pub const MAX_PANICS_WITHOUT_A_TAKE: u32 = 3;

/// Whether a worker is serving, and what did not reach it. Cheap to clone; any thread.
#[derive(Clone, Debug, Default)]
pub struct WorkerHealth(Arc<HealthState>);

#[derive(Debug, Default)]
struct HealthState {
    stopped: AtomicBool,
    dropped: AtomicU64,
}

impl WorkerHealth {
    /// Whether the worker thread is still taking inputs.
    pub fn is_running(&self) -> bool {
        !self.0.stopped.load(Ordering::Acquire)
    }

    /// Inputs sent after the worker stopped (hotkey events included), which nothing acted on.
    pub fn dropped_inputs(&self) -> u64 {
        self.0.dropped.load(Ordering::Acquire)
    }

    fn dropped(&self) {
        self.0.dropped.fetch_add(1, Ordering::AcqRel);
    }
}

/// Something for the chain.
#[derive(Debug)]
#[non_exhaustive]
pub enum Input {
    /// Mic audio in the canonical format ([`MicPath`](crate::mic::MicPath)'s output).
    Audio {
        /// 16 kHz mono.
        samples: Vec<f32>,
        /// Host time of the first sample.
        host_time_ns: u64,
        /// Device frames the capture ring dropped before it.
        dropped_frames: u64,
    },
    /// A hotkey event.
    Hotkey(HotkeyEvent),
    /// A VAD was installed or went away.
    SetVad(Vad),
    /// New settings.
    SetSettings(Box<DictationSettings>),
    /// The mic stream stopped.
    StreamEnded,
    /// Finish and hand the chain back ([`DictationWorker::stop`]).
    Stop,
}

/// Why an input was not delivered: the worker has stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkerGone;

impl std::fmt::Display for WorkerGone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the dictation worker has stopped")
    }
}

impl std::error::Error for WorkerGone {}

/// A running chain.
pub struct DictationWorker {
    tx: Sender<Input>,
    thread: JoinHandle<DictationChain>,
    health: WorkerHealth,
}

impl DictationWorker {
    /// **Worker** (any thread but realtime). Starts a thread that owns `chain`. `clock` is the
    /// platform clock the chain uses, for the tail deadline.
    pub fn spawn(chain: DictationChain, clock: Arc<dyn Clock>) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let health = WorkerHealth::default();
        let running = health.clone();
        let thread = thread::Builder::new()
            .name("ink-dictation".into())
            .spawn(move || {
                let chain = run(chain, &rx, clock.as_ref());
                // Before the receiver drops, so a sender that sees the send fail also sees this.
                running.0.stopped.store(true, Ordering::Release);
                drop(rx);
                chain
            })?;
        Ok(Self { tx, thread, health })
    }

    /// Whether the worker is serving, and what did not reach it.
    pub fn health(&self) -> WorkerHealth {
        self.health.clone()
    }

    /// A sender for the pump and the shell. A failed send on it means the worker has stopped
    /// (see [`health`](Self::health)).
    pub fn sender(&self) -> Sender<Input> {
        self.tx.clone()
    }

    /// Queues an input. After the worker stops it returns [`WorkerGone`] and counts the input
    /// in [`WorkerHealth::dropped_inputs`].
    pub fn send(&self, input: Input) -> Result<(), WorkerGone> {
        self.tx.send(input).map_err(|_| {
            self.health.dropped();
            WorkerGone
        })
    }

    /// The sink to hand the platform's [`HotkeySource::start`](ink_core::HotkeySource::start).
    /// **Callback thread**: it only enqueues (and never logs, which could block the OS's event
    /// thread). After the worker stops, an event cannot be acted on: it is counted in
    /// [`WorkerHealth::dropped_inputs`], and the shell, told by
    /// [`WorkerFailed`](crate::events::DictationEvent::WorkerFailed), stops the hotkey or starts
    /// a new worker.
    pub fn hotkey_sink(&self) -> EventSink<HotkeyEvent> {
        let tx = self.tx.clone();
        let health = self.health.clone();
        Arc::new(move |event| {
            if tx.send(Input::Hotkey(event)).is_err() {
                health.dropped();
            }
        })
    }

    /// Processes what is queued, ends any take in progress as if the stream had stopped, and
    /// returns the chain (also after the worker stopped itself). `Err` only if the thread
    /// panicked outside the per-input boundary.
    pub fn stop(self) -> thread::Result<DictationChain> {
        // If the thread is already gone, `join` reports how it ended.
        let _ = self.tx.send(Input::Stop);
        self.thread.join()
    }
}

/// What one input asks of the loop.
enum Step {
    Continue,
    Stop,
}

fn dispatch(chain: &mut DictationChain, input: Option<Input>) -> Step {
    match input {
        Some(Input::Audio {
            samples,
            host_time_ns,
            dropped_frames,
        }) => chain.push_audio(&samples, host_time_ns, dropped_frames),
        Some(Input::Hotkey(event)) => chain.hotkey(event),
        Some(Input::SetVad(vad)) => chain.set_vad(vad),
        Some(Input::SetSettings(settings)) => chain.set_settings(*settings),
        Some(Input::StreamEnded) => chain.stream_ended(),
        Some(Input::Stop) | None => {
            chain.stream_ended();
            return Step::Stop;
        }
    }
    Step::Continue
}

/// What the loop woke for.
enum Work {
    Input(Option<Input>),
    Tick,
}

fn run(mut chain: DictationChain, rx: &Receiver<Input>, clock: &dyn Clock) -> DictationChain {
    let mut panics = 0u32;
    let mut takes_at_last_panic = chain.completed_takes();
    loop {
        let work = match chain.deadline_ns() {
            None => Work::Input(rx.recv().ok()),
            Some(deadline) => {
                let wait = Duration::from_nanos(deadline.saturating_sub(clock.now_ns()));
                match rx.recv_timeout(wait) {
                    Ok(input) => Work::Input(Some(input)),
                    Err(RecvTimeoutError::Timeout) => Work::Tick,
                    Err(RecvTimeoutError::Disconnected) => Work::Input(None),
                }
            }
        };
        // A stop that panics still stops: `stop` is waiting on this thread.
        let stopping = matches!(work, Work::Input(Some(Input::Stop) | None));
        // The chain is left in whatever state the panic interrupted, which is why
        // `recover_from_panic` rebuilds everything mid-take rather than trusting it: that is what
        // makes asserting unwind safety sound here.
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| match work {
            Work::Tick => {
                chain.tick();
                Step::Continue
            }
            Work::Input(input) => dispatch(&mut chain, input),
        }));
        match outcome {
            Ok(Step::Continue) => {}
            Ok(Step::Stop) => return chain,
            Err(_) => {
                // The payload is not logged: a panic message could quote what was said (I5).
                if chain.completed_takes() != takes_at_last_panic {
                    panics = 0;
                }
                panics += 1;
                takes_at_last_panic = chain.completed_takes();
                let recovered = !stopping && panics < MAX_PANICS_WITHOUT_A_TAKE;
                log::error!(
                    "dictation worker: a stage panicked ({panics} in a row); {}",
                    if recovered { "recovered" } else { "stopping" }
                );
                // Emitting runs the shell's sink; if that panics too, stop.
                let reported =
                    panic::catch_unwind(AssertUnwindSafe(|| chain.recover_from_panic(recovered)));
                if !recovered || reported.is_err() {
                    return chain;
                }
            }
        }
    }
}
