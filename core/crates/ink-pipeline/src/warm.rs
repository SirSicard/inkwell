//! Warming the dictation engine when a take starts, so the first dictation after a quiet spell is
//! as quick as any other.
//!
//! With ggml's Metal residency off (the Mac shell turns it off: it cost idle CPU), the first
//! decode after a few idle minutes was measured once at about 110 ms slower than a warm one. A take
//! starts well before its final is needed (the user is still speaking), so a short decode started
//! at the take's start pays that cost while they speak.
//!
//! **What it never does:**
//! - **Delay the take.** The engine the chain calls ([`EngineWarmer::engine`]) marks the engine
//!   busy and cancels a warm-up still running before it transcribes, and a warm-up never starts
//!   once a take has begun its decode (both under one lock). What a take can wait for is the
//!   warm-up's step in progress (an engine checks its token between steps), never its whole
//!   decode.
//! - **Put words anywhere.** The warm-up decodes [`WARM_AUDIO`] samples of digital silence, and
//!   its answer is dropped unread: Qwen3-ASR invents text on silence, which is why only VAD speech
//!   ever reaches it from a take. It is not a transcription, so it skips the gain stage.
//! - **Run while the engine is warm.** Only after [`WARM_AFTER_IDLE`] without a decode (a take's
//!   or a warm-up's).
//!
//! **Threads.** One thread, `ink-warm`, waits on a channel and exists while the warmer does:
//! nothing ticks. [`key_down`](EngineWarmer::key_down) only sends (any thread but realtime).

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ink_core::{
    CancelToken, Channel, Clock, EngineError, EngineInfo, OfflineEngine, TranscribeOptions,
    Transcript,
};

/// Idle time after which a take's start warms the engine.
pub const WARM_AFTER_IDLE: Duration = Duration::from_secs(30);

/// The longest a warm-up may run. It is cancelled sooner by a take's own decode.
pub const WARM_BUDGET: Duration = Duration::from_secs(2);

/// What a warm-up decodes: half a second of digital silence at 16 kHz.
pub const WARM_AUDIO: usize = 8_000;

/// Host time 0 means "never decoded".
const NEVER: u64 = 0;

struct Shared {
    engine: Arc<dyn OfflineEngine>,
    clock: Arc<dyn Clock>,
    after_idle: Duration,
    /// Host time a decode last started (a take's) or ended (a warm-up's); [`NEVER`] before the
    /// first. Read and written under `running`'s lock where it decides anything.
    last_decode_ns: AtomicU64,
    /// The warm-up in progress, to cancel.
    running: Mutex<Option<CancelToken>>,
    warmups: AtomicU64,
    yielded: AtomicU64,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Option<CancelToken>> {
        // A token swap: consistent at every step.
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn idle(&self, now_ns: u64) -> bool {
        let last = self.last_decode_ns.load(Ordering::Acquire);
        last == NEVER || now_ns.saturating_sub(last) >= nanos(self.after_idle)
    }

    fn mark(&self) {
        // Never 0, which reads as "never decoded".
        self.last_decode_ns
            .store(self.clock.now_ns().max(1), Ordering::Release);
    }
}

fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

/// Warms an engine at a take's start. See the module docs.
pub struct EngineWarmer {
    shared: Arc<Shared>,
    tx: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl EngineWarmer {
    /// Starts `ink-warm` for `engine`, warming after `after_idle` ([`WARM_AFTER_IDLE`]) without a
    /// decode, idle measured on `clock`.
    pub fn start(
        engine: Arc<dyn OfflineEngine>,
        clock: Arc<dyn Clock>,
        after_idle: Duration,
    ) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            engine,
            clock,
            after_idle,
            last_decode_ns: AtomicU64::new(NEVER),
            running: Mutex::new(None),
            warmups: AtomicU64::new(0),
            yielded: AtomicU64::new(0),
        });
        let (tx, rx) = mpsc::channel();
        let thread = {
            let shared = shared.clone();
            thread::Builder::new()
                .name("ink-warm".into())
                .spawn(move || run(&shared, &rx))?
        };
        Ok(Self {
            shared,
            tx: Some(tx),
            thread: Some(thread),
        })
    }

    /// The engine a chain should call: the warmed one, which comes first. Cheap to clone.
    pub fn engine(&self) -> Arc<dyn OfflineEngine> {
        Arc::new(TakesFirst {
            shared: self.shared.clone(),
        })
    }

    /// **Any thread but realtime.** A take has started: warm the engine if it has been idle.
    /// Returns whether a warm-up was asked for (it may still be skipped, if a take's decode starts
    /// first).
    pub fn key_down(&self) -> bool {
        if !self.shared.idle(self.shared.clock.now_ns()) {
            return false;
        }
        self.tx.as_ref().is_some_and(|tx| tx.send(()).is_ok())
    }

    /// Warm-ups run to their end (answered, failed or cancelled) so far.
    pub fn warmups(&self) -> u64 {
        self.shared.warmups.load(Ordering::Acquire)
    }

    /// Warm-ups a take's decode cancelled so far.
    pub fn yielded(&self) -> u64 {
        self.shared.yielded.load(Ordering::Acquire)
    }

    /// Cancels a warm-up in progress and ends the thread.
    pub fn stop(mut self) {
        self.shut();
    }

    fn shut(&mut self) {
        if let Some(token) = self.shared.lock().as_ref() {
            token.cancel();
        }
        drop(self.tx.take());
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            log::error!("the warm-up thread panicked");
        }
    }
}

impl Drop for EngineWarmer {
    fn drop(&mut self) {
        self.shut();
    }
}

fn run(shared: &Shared, rx: &Receiver<()>) {
    while rx.recv().is_ok() {
        // Requests that queued up meanwhile are one warm-up.
        while rx.try_recv().is_ok() {}
        let token = {
            let mut running = shared.lock();
            if !shared.idle(shared.clock.now_ns()) {
                // A take's decode began (or a warm-up just ran): the engine is warm, or busy.
                continue;
            }
            let token = Instant::now()
                .checked_add(WARM_BUDGET)
                .map_or_else(CancelToken::new, CancelToken::with_deadline);
            *running = Some(token.clone());
            token
        };
        let started = Instant::now();
        let options = TranscribeOptions {
            channel: Channel::Mic,
            context: None,
            cancel: token,
        };
        // The answer is dropped unread: see the module docs. A panic in the engine costs the
        // warm-up, never the thread (the next take must still be warmed).
        let silence = [0.0f32; WARM_AUDIO];
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            shared.engine.transcribe(&silence, &options).map(drop)
        }));
        {
            let mut running = shared.lock();
            *running = None;
            shared.mark();
        }
        shared.warmups.fetch_add(1, Ordering::AcqRel);
        let ms = started.elapsed().as_millis();
        match outcome {
            Ok(Ok(())) => log::info!("dictation engine warmed in {ms} ms"),
            Ok(Err(EngineError::Cancelled)) => {
                log::info!("dictation engine warm-up gave way after {ms} ms");
            }
            // The error names the engine, never any text.
            Ok(Err(e)) => log::warn!("dictation engine warm-up failed after {ms} ms: {e}"),
            Err(_) => log::error!("dictation engine warm-up panicked after {ms} ms"),
        }
    }
}

/// The engine the chain calls: marks the engine busy, cancels a warm-up in progress, then decodes.
struct TakesFirst {
    shared: Arc<Shared>,
}

impl OfflineEngine for TakesFirst {
    fn info(&self) -> EngineInfo {
        self.shared.engine.info()
    }

    fn transcribe(
        &self,
        audio: &[f32],
        options: &TranscribeOptions,
    ) -> Result<Transcript, EngineError> {
        {
            let running = self.shared.lock();
            self.shared.mark();
            if let Some(token) = running.as_ref() {
                token.cancel();
                self.shared.yielded.fetch_add(1, Ordering::AcqRel);
            }
        }
        let result = self.shared.engine.transcribe(audio, options);
        self.shared.mark();
        result
    }
}
