//! The thread that owns the dictation chain, fed through a bounded [`Mailbox`].
//!
//! It follows `ink-pipeline`'s `worker` (same loop, same panic policy) with one change: the pump's
//! audio goes through a bounded queue. A chain stuck in a slow engine then costs dropped and
//! reported audio (`audio.dropped`), never unbounded memory or a stalled pump. Hotkey events and
//! everything else stay unbounded and in order with the audio.
//!
//! The thread waits on the queue and has no timer: the chain's one deadline (a tail whose audio
//! stopped arriving, [`DictationChain::deadline_ns`]) becomes the wait's timeout, and
//! [`DictationChain::tick`] runs when it passes.
//!
//! **Panics.** Each input runs behind a panic boundary. A panic costs the take in progress; the
//! chain goes back to idle and the shell gets `dictation.worker_failed` with `recovered: true`.
//! After [`MAX_PANICS_WITHOUT_A_TAKE`] panics with no take completing between them, the worker
//! stops, and the event says `recovered: false, unbind_hotkey: true`: **the shell must unbind the
//! hotkey**, because key presses now reach nothing. What is sent after that is refused and
//! counted.

use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ink_core::{Clock, EventSink, HotkeyEvent};
use ink_pipeline::chain::{DictationChain, DictationSettings};
use ink_pipeline::gain_stage::Vad;
use ink_pipeline::worker::MAX_PANICS_WITHOUT_A_TAKE;

use crate::events;
use crate::hub::Events;
use crate::mailbox::{Item, Mailbox, Pop, Pushed};

/// Mic audio for the chain: 16 kHz mono, as the pump hands it on.
#[derive(Debug)]
pub struct Block {
    /// The samples.
    pub samples: Vec<f32>,
    /// Host time of the first.
    pub host_time_ns: u64,
    /// Device frames the capture ring dropped before it.
    pub dropped_frames: u64,
}

/// Anything else for the chain.
#[derive(Debug)]
#[non_exhaustive]
pub enum Input {
    /// A hotkey edge.
    Hotkey(HotkeyEvent),
    /// An edge of the voice-edit key.
    EditHotkey(HotkeyEvent),
    /// The mic was let go of while idle ([`DictationChain::mic_closed`]).
    MicClosed,
    /// A VAD was installed or went away.
    SetVad(Vad),
    /// New settings.
    SetSettings(Box<DictationSettings>),
    /// The mic stream stopped.
    StreamEnded,
    /// Finish what is queued and end the thread.
    Stop,
}

/// The worker has stopped: nothing will act on this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkerGone;

/// Where the pump, the hotkey and the shell send dictation input. Cheap to clone; it holds only
/// the queue, so a clone kept elsewhere never keeps the chain alive.
#[derive(Clone)]
pub struct DictationInbox {
    mailbox: Arc<Mailbox<Block, Input>>,
    events: Events,
}

impl DictationInbox {
    /// **Pump.** Queues mic audio. Never waits: a full queue drops the block and counts it, and
    /// the stretch dropped is reported (`audio.dropped`) when the queue takes audio again.
    pub fn push_audio(&self, block: Block) -> Pushed {
        let n = block.samples.len();
        let pushed = self.mailbox.push_audio(block, n);
        if let Pushed::Queued {
            after_drop: Some(o),
        } = pushed
        {
            self.events.emit(dropped_event("dictation", Some("mic"), o));
        }
        pushed
    }

    /// Queues anything but audio. [`WorkerGone`] once the worker has stopped.
    pub fn send(&self, input: Input) -> Result<(), WorkerGone> {
        self.mailbox.push(input).map_err(|_| WorkerGone)
    }

    /// The sink for the platform's hotkey source. **Callback thread:** it only enqueues and never
    /// logs. After the worker stops, events are refused (the shell was told to unbind the key).
    pub fn hotkey_sink(&self) -> EventSink<HotkeyEvent> {
        let mailbox = self.mailbox.clone();
        Arc::new(move |event| {
            let _ = mailbox.push(Input::Hotkey(event));
        })
    }

    /// Blocks the pump dropped so far.
    pub fn dropped(&self) -> crate::mailbox::Overflow {
        self.mailbox.dropped()
    }

    /// Audio blocks waiting for the worker now.
    pub fn queued_audio(&self) -> usize {
        self.mailbox.queued_audio()
    }
}

/// A running dictation worker: its thread, owned by the core, which stops it.
pub struct DictationWorker {
    inbox: DictationInbox,
    thread: JoinHandle<DictationChain>,
}

impl DictationWorker {
    /// **Worker.** Starts the thread that owns `chain`. `clock` is the chain's clock, for the tail
    /// deadline; `capacity` bounds the queued audio blocks.
    pub fn spawn(
        chain: DictationChain,
        clock: Arc<dyn Clock>,
        events: Events,
        capacity: usize,
    ) -> io::Result<Self> {
        let mailbox = Arc::new(Mailbox::new(capacity));
        let inbox = mailbox.clone();
        let out = events.clone();
        let thread = thread::Builder::new()
            .name("ink-dictation".into())
            .spawn(move || {
                let chain = run(chain, &inbox, clock.as_ref(), &out);
                // Closed before the thread ends, so a sender sees `Closed` from here on.
                inbox.close();
                chain
            })?;
        Ok(Self {
            inbox: DictationInbox { mailbox, events },
            thread,
        })
    }

    /// A sender for the pump, the hotkey and the shell.
    pub fn inbox(&self) -> DictationInbox {
        self.inbox.clone()
    }

    /// Processes what is queued, ends a take in progress as if the stream had stopped, and
    /// returns the chain. `Err` only if the thread panicked outside its per-input boundary.
    pub fn stop(self) -> thread::Result<DictationChain> {
        let _ = self.inbox.mailbox.push(Input::Stop);
        let chain = self.thread.join();
        if let Some(o) = self.inbox.mailbox.close() {
            self.inbox
                .events
                .emit(dropped_event("dictation", Some("mic"), o));
        }
        chain
    }
}

/// `audio.dropped`.
pub fn dropped_event(
    chain: &str,
    channel: Option<&str>,
    o: crate::mailbox::Overflow,
) -> serde_json::Value {
    log::warn!(
        "{chain} queue full: {} blocks ({} samples) of audio dropped",
        o.blocks,
        o.samples
    );
    events::event(
        "audio.dropped",
        &[
            ("chain", Some(chain.into())),
            ("channel", channel.map(Into::into)),
            ("blocks", Some(o.blocks.into())),
            ("samples", Some(o.samples.into())),
        ],
    )
}

enum Step {
    Continue,
    Stop,
}

enum Work {
    /// An item, or `None` when the mailbox closed: stop.
    Item(Option<Item<Block, Input>>),
    Tick,
}

fn dispatch(chain: &mut DictationChain, item: Option<Item<Block, Input>>) -> Step {
    match item {
        Some(Item::Audio(b, _)) => chain.push_audio(&b.samples, b.host_time_ns, b.dropped_frames),
        Some(Item::Other(Input::Hotkey(e))) => chain.hotkey(e),
        Some(Item::Other(Input::EditHotkey(e))) => chain.edit_hotkey(e),
        Some(Item::Other(Input::MicClosed)) => chain.mic_closed(),
        Some(Item::Other(Input::SetVad(v))) => chain.set_vad(v),
        Some(Item::Other(Input::SetSettings(s))) => chain.set_settings(*s),
        Some(Item::Other(Input::StreamEnded)) => chain.stream_ended(),
        Some(Item::Other(Input::Stop)) | None => {
            chain.stream_ended();
            return Step::Stop;
        }
    }
    Step::Continue
}

/// The host-time deadline as an instant on this thread's clock.
fn instant(deadline_ns: u64, clock: &dyn Clock) -> Instant {
    Instant::now() + Duration::from_nanos(deadline_ns.saturating_sub(clock.now_ns()))
}

fn run(
    mut chain: DictationChain,
    mailbox: &Mailbox<Block, Input>,
    clock: &dyn Clock,
    events: &Events,
) -> DictationChain {
    let mut panics = 0u32;
    let mut takes_at_last_panic = chain.completed_takes();
    loop {
        let deadline = chain.deadline_ns().map(|d| instant(d, clock));
        let work = match mailbox.pop(deadline) {
            Pop::Item(item) => Work::Item(Some(item)),
            Pop::TimedOut => Work::Tick,
            Pop::Closed => Work::Item(None),
        };
        let stopping = matches!(work, Work::Item(Some(Item::Other(Input::Stop)) | None));
        // As in ink-pipeline's worker: `recover_from_panic` rebuilds everything mid-take, so the
        // state a panic interrupted is never trusted again.
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| match work {
            Work::Tick => {
                chain.tick();
                Step::Continue
            }
            Work::Item(item) => dispatch(&mut chain, item),
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
                // The chain reports it through its own sink (dictation.worker_failed, with
                // unbind_hotkey set when not recovered). If that panics too, the core says it.
                let reported =
                    panic::catch_unwind(AssertUnwindSafe(|| chain.recover_from_panic(recovered)));
                if reported.is_err() {
                    log::error!("dictation worker: the event sink panicked; stopping");
                    events.emit(events::worker_failed(false));
                    return chain;
                }
                if !recovered {
                    return chain;
                }
            }
        }
    }
}
