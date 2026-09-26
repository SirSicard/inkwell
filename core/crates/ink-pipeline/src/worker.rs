//! The thread that owns a dictation chain.
//!
//! Everything reaches the chain through one queue, in order: audio from the pump, hotkey events
//! from the platform's callback thread, and settings from the shell. The hotkey callback only
//! enqueues (the OS disables an event tap that is slow), and the pump never waits on the chain,
//! whose engine call can take a second. The queue is unbounded: a dictation's audio is seconds.
//!
//! The thread blocks on the queue; it has no timer. The one deadline, a tail whose audio stopped
//! arriving, becomes the timeout of that wait.

use std::io;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ink_core::{Clock, EventSink, HotkeyEvent};

use crate::chain::{DictationChain, DictationSettings};
use crate::gain_stage::Vad;

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
}

impl DictationWorker {
    /// **Worker** (any thread but realtime). Starts a thread that owns `chain`. `clock` is the
    /// platform clock the chain uses, for the tail deadline.
    pub fn spawn(chain: DictationChain, clock: Arc<dyn Clock>) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("ink-dictation".into())
            .spawn(move || run(chain, &rx, clock.as_ref()))?;
        Ok(Self { tx, thread })
    }

    /// A sender for the pump and the shell.
    pub fn sender(&self) -> Sender<Input> {
        self.tx.clone()
    }

    /// Queues an input.
    pub fn send(&self, input: Input) -> Result<(), WorkerGone> {
        self.tx.send(input).map_err(|_| WorkerGone)
    }

    /// The sink to hand the platform's [`HotkeySource::start`](ink_core::HotkeySource::start).
    /// **Callback thread**: it only enqueues. After the worker stops it drops the event, since
    /// there is no take left for it to act on.
    pub fn hotkey_sink(&self) -> EventSink<HotkeyEvent> {
        let tx = self.tx.clone();
        Arc::new(move |event| {
            let _ = tx.send(Input::Hotkey(event));
        })
    }

    /// Processes what is queued, ends any take in progress as if the stream had stopped, and
    /// returns the chain. `Err` if the thread panicked.
    pub fn stop(self) -> thread::Result<DictationChain> {
        // If the thread is already gone, `join` reports how it ended.
        let _ = self.tx.send(Input::Stop);
        self.thread.join()
    }
}

fn run(mut chain: DictationChain, rx: &Receiver<Input>, clock: &dyn Clock) -> DictationChain {
    loop {
        let input = match chain.deadline_ns() {
            None => rx.recv().ok(),
            Some(deadline) => {
                let wait = Duration::from_nanos(deadline.saturating_sub(clock.now_ns()));
                match rx.recv_timeout(wait) {
                    Ok(input) => Some(input),
                    Err(RecvTimeoutError::Timeout) => {
                        chain.tick();
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => None,
                }
            }
        };
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
                return chain;
            }
        }
    }
}
