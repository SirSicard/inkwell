//! Live partials from an offline engine, by re-decoding a trailing window: [`TrailingWindow`], the
//! live-partials engine for a model the core loads itself (Parakeet on sherpa-onnx, on Windows).
//! The Mac's Parakeet runs the same scheme in Swift (`mac/Sources/AppleEngines/Parakeet/`,
//! `LiveWindow.swift` and `ParakeetLiveEngine.swift`); this is its port, with the same tuning.
//!
//! # The scheme
//!
//! The stream keeps the audio of the utterance not yet settled (plus a little silence in front of
//! it). Every hop of new audio it decodes all of that again with the offline engine, which sees the
//! whole utterance each time: the words shown as a partial are the engine's words for what has
//! been said so far. An utterance is settled (a final) when
//!
//! - a pause follows its last word: the decoded words end at least `pause` before the window's
//!   end, or
//! - it has grown to `max_utterance` of audio: the words that end before the last `tail_guard` are
//!   settled at a word boundary, and the rest stay pending.
//!
//! Both are measured in samples of audio, never with a timer, so how the audio is pushed cannot
//! postpone them. A final carries only its own words and a partial only the words not settled yet.
//! A partial leaves out the words that end in the newest `hide_newest` (0.16 s) of the decoded
//! audio, the words the next decode most often changes (the owner's choice, 2026-09-27); finals keep
//! every word.
//!
//! The engine places its words: one [`TimedText`] segment per word, in ms from the start of the
//! audio it was given (as [`crate::sherpa`] returns them). An engine that returns one segment for
//! the whole window still works, but settles only by length.
//!
//! # Threads
//!
//! Each stream has a decode thread (`ink-live`). [`EngineStream::push`] only appends to the
//! stream's buffer under its lock and wakes that thread; it never waits for a decode. The thread
//! decodes one window at a time, and when a decode ends it takes the newest window if a hop of audio
//! arrived meanwhile, so a slow engine skips hypotheses (partials are ephemeral) rather than
//! queueing them.
//!
//! - A backlog of [`STALL_AFTER`] while a decode runs is reported once as [`AsrEvent::Stalled`].
//!   A decode so slow that the buffer outgrew its cap (30 s) lets the oldest audio go unheard; that
//!   is reported the same way, once per such decode, with the seconds let go of (the final pass
//!   still has them). RAM holds seconds, never the session.
//! - Events leave one at a time under the stream's send lock, in the order they happened. Once the
//!   stream is dropped none are sent: a decode under way ends on its own and its words go nowhere.
//! - [`EngineStream::finish`] decodes what is left, settles all of it, and returns once its events
//!   are sent.
//! - A decode that fails ends the stream: every later push, and the finish, return its error.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};

use ink_core::{
    AsrEvent, CANONICAL_RATE, CancelToken, Channel, EngineError, EngineInfo, EngineStream,
    EventSink, OfflineEngine, StreamingEngine, TimedText, TranscribeOptions, Transcript,
};

use crate::lock;

/// Samples per second: the only rate the core hands an engine.
const RATE: usize = CANONICAL_RATE as usize;

/// Samples per millisecond.
const PER_MS: usize = RATE / 1_000;

/// A backlog this long (3 s) while a decode runs is reported as a stall.
const STALL_AFTER: usize = 3 * RATE;

/// The scheme's tuning, in samples: the Mac's (`LiveWindowConfig`), unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LiveConfig {
    /// New audio between two decodes: how often the partial can change (0.5 s).
    hop: usize,
    /// A gap this long after the last decoded word settles the utterance (0.8 s).
    pause: usize,
    /// An utterance this long is settled at a word boundary, pause or not (12 s).
    max_utterance: usize,
    /// When a long utterance is settled, words ending in its last stretch this long stay pending,
    /// so a word still being said is not cut (1 s).
    tail_guard: usize,
    /// Audio kept when a decode hears no words (3 s): Parakeet often hears nothing in the first
    /// second or so of speech in a short window, and keeping only 1 s dropped that speech.
    keep_silence: usize,
    /// The shortest window worth decoding (0.3 s).
    minimum: usize,
    /// A settled utterance keeps this much audio after its last word, so the next window does not
    /// start inside that word's tail (0.2 s).
    after_word: usize,
    /// A partial does not show words that end within this much of the decoded window's end
    /// (0.16 s).
    hide_newest: usize,
    /// The most audio kept (30 s); past it the oldest goes, unheard.
    max_buffer: usize,
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self {
            hop: 8_000,
            pause: 12_800,
            max_utterance: 192_000,
            tail_guard: 16_000,
            keep_silence: 48_000,
            minimum: 4_800,
            after_word: 3_200,
            hide_newest: 2_560,
            max_buffer: 480_000,
        }
    }
}

/// A decoded word, in samples from the start of the window it was decoded from.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Word {
    text: String,
    start: usize,
    end: usize,
}

/// A stretch of the stream to decode: its audio, and its first sample's place in the stream.
#[derive(Clone, Debug, PartialEq)]
struct Window {
    samples: Vec<f32>,
    start: usize,
}

impl Window {
    fn end(&self) -> usize {
        self.start + self.samples.len()
    }
}

/// What the scheme sends after a decode.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Output {
    /// The words not settled yet, replacing the last partial ("" clears it).
    Partial(String),
    /// Settled words, in ms from the stream's first sample.
    Final(TimedText),
}

/// The scheme's state for one stream: decisions only, no engine and no threads.
#[derive(Debug)]
struct LiveWindow {
    config: LiveConfig,
    /// Audio not settled yet, from `buffer_start`.
    buffer: VecDeque<f32>,
    /// The stream sample `buffer[0]` is.
    buffer_start: usize,
    /// Samples received so far.
    received: usize,
    /// `received` when the last window was taken.
    taken_at: usize,
    /// The partial last sent.
    last_partial: String,
    /// Samples let go of unheard because the buffer outgrew `max_buffer`.
    dropped_unheard: usize,
    /// `dropped_unheard` when [`take_unheard_drops`](Self::take_unheard_drops) last reported it.
    reported_unheard: usize,
}

impl LiveWindow {
    fn new(config: LiveConfig) -> Self {
        Self {
            config,
            buffer: VecDeque::new(),
            buffer_start: 0,
            received: 0,
            taken_at: 0,
            last_partial: String::new(),
            dropped_unheard: 0,
            reported_unheard: 0,
        }
    }

    /// Takes the stream's next samples.
    fn append(&mut self, samples: &[f32]) {
        self.buffer.extend(samples);
        self.received += samples.len();
        if self.buffer.len() > self.config.max_buffer {
            let to = self.buffer_start + self.buffer.len() - self.config.max_buffer;
            self.drop_to(to, true);
        }
    }

    /// Samples let go of unheard since the last call: asked once per decode, so a stretch of
    /// pushes past the cap is reported once, never per push.
    fn take_unheard_drops(&mut self) -> usize {
        let n = self.dropped_unheard - self.reported_unheard;
        self.reported_unheard = self.dropped_unheard;
        n
    }

    /// Audio received since the last window was taken: what a partial does not show yet.
    fn backlog(&self) -> usize {
        self.received - self.taken_at
    }

    /// Whether enough new audio has arrived for another decode.
    fn wants_decode(&self) -> bool {
        self.backlog() >= self.config.hop && self.buffer.len() >= self.config.minimum
    }

    /// The window to decode now: every sample not settled yet.
    fn take_window(&mut self) -> Window {
        self.taken_at = self.received;
        Window {
            samples: self.buffer.iter().copied().collect(),
            start: self.buffer_start,
        }
    }

    /// Applies the decode of `window`, the window last taken. Audio that arrived meanwhile stays
    /// in the buffer for the next one.
    fn apply(&mut self, words: &[Word], window: &Window) -> Vec<Output> {
        // Audio let go of while the window decoded (the buffer outgrew its cap): the decode is
        // stale.
        if window.start != self.buffer_start {
            return Vec::new();
        }
        let len = window.samples.len();
        let Some(last) = words.last() else {
            // Nothing said: keep only the last moment of it, in front of whatever comes next.
            let keep_from = window.end().saturating_sub(self.config.keep_silence);
            self.drop_to(self.buffer_start.max(keep_from), false);
            return self.partial(String::new());
        };
        if len.saturating_sub(last.end) >= self.config.pause {
            // A pause after the last word: the utterance is settled.
            self.drop_to(
                window.start + (last.end + self.config.after_word).min(len),
                false,
            );
            return self.settle(words, window, String::new());
        }
        if len >= self.config.max_utterance {
            // Long and unbroken: settle up to a word boundary, leaving the words still being said.
            let limit = len - self.config.tail_guard;
            let n = words.iter().take_while(|w| w.end <= limit).count();
            let (settled, rest) = words.split_at(n);
            return match (settled.last(), rest.first()) {
                (Some(last_settled), Some(first_pending)) => {
                    // Cut in the gap between the two words, so neither is split.
                    let cut = (last_settled.end + first_pending.start) / 2;
                    self.drop_to(window.start + cut, false);
                    let shown = self.shown(rest, window);
                    self.settle(settled, window, shown)
                }
                _ => {
                    // One word longer than the guard, or every word before it: settle everything.
                    self.drop_to(
                        window.start + (last.end + self.config.after_word).min(len),
                        false,
                    );
                    self.settle(words, window, String::new())
                }
            };
        }
        let shown = self.shown(words, window);
        self.partial(shown)
    }

    /// The window for the stream's end: everything not settled, when there is enough to decode.
    fn take_last_window(&mut self) -> Option<Window> {
        self.taken_at = self.received;
        (self.buffer.len() >= self.config.minimum).then(|| Window {
            samples: self.buffer.iter().copied().collect(),
            start: self.buffer_start,
        })
    }

    /// Applies the last window's decode: every word is settled.
    fn apply_last(&mut self, words: &[Word], window: &Window) -> Vec<Output> {
        if window.start != self.buffer_start {
            return Vec::new();
        }
        self.drop_to(window.end(), false);
        if words.is_empty() {
            return self.partial(String::new());
        }
        self.settle(words, window, String::new())
    }

    /// The stream's end with too little audio left to decode: nothing more is settled, and the
    /// partial is cleared.
    fn end_without_window(&mut self) -> Vec<Output> {
        self.drop_to(self.buffer_start + self.buffer.len(), false);
        self.partial(String::new())
    }

    /// A final of `words`, then `partial` if it is not what was sent last.
    fn settle(&mut self, words: &[Word], window: &Window, partial: String) -> Vec<Output> {
        let mut out = vec![Output::Final(segment(words, window))];
        out.extend(self.partial(partial));
        out
    }

    /// The words of a partial: those not ending in the newest `hide_newest` of `window`.
    fn shown(&self, words: &[Word], window: &Window) -> String {
        let limit = window.samples.len().saturating_sub(self.config.hide_newest);
        words
            .iter()
            .take_while(|w| w.end <= limit)
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The partial to send: `text`, unless it is what was sent last.
    fn partial(&mut self, text: String) -> Vec<Output> {
        if text == self.last_partial {
            return Vec::new();
        }
        self.last_partial.clone_from(&text);
        vec![Output::Partial(text)]
    }

    fn drop_to(&mut self, sample: usize, unheard: bool) {
        let n = sample
            .saturating_sub(self.buffer_start)
            .min(self.buffer.len());
        self.buffer.drain(..n);
        self.buffer_start += n;
        if unheard {
            self.dropped_unheard += n;
        }
    }
}

/// `words` of `window` as a final, in ms from the stream's first sample.
fn segment(words: &[Word], window: &Window) -> TimedText {
    let ms = |sample: usize| ((window.start + sample) / PER_MS) as u64;
    let start = words.first().map_or(0, |w| w.start);
    let end = words.last().map_or(0, |w| w.end).max(start);
    TimedText {
        start_ms: ms(start),
        end_ms: ms(end),
        text: words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// A transcript's words, in samples, kept inside the `len` samples decoded.
fn words_of(transcript: &Transcript, len: usize) -> Vec<Word> {
    transcript
        .segments
        .iter()
        .filter(|s| !s.text.trim().is_empty())
        .map(|s| {
            let at =
                |ms: u64| usize::try_from(ms).map_or(len, |ms| ms.saturating_mul(PER_MS).min(len));
            let start = at(s.start_ms);
            Word {
                text: s.text.trim().to_owned(),
                start,
                end: at(s.end_ms).max(start),
            }
        })
        .collect()
}

/// Live partials by re-decoding a trailing window with an offline engine. See the module docs.
pub struct TrailingWindow {
    engine: Arc<dyn OfflineEngine>,
    info: EngineInfo,
    config: LiveConfig,
}

impl TrailingWindow {
    /// Live partials from `engine`, which places its words (see the module docs), reported as
    /// `info`.
    pub fn new(engine: Arc<dyn OfflineEngine>, info: EngineInfo) -> Self {
        Self {
            engine,
            info,
            config: LiveConfig::default(),
        }
    }
}

impl StreamingEngine for TrailingWindow {
    fn info(&self) -> EngineInfo {
        self.info.clone()
    }

    fn open_stream(
        &self,
        channel: Channel,
        events: EventSink<AsrEvent>,
    ) -> Result<Box<dyn EngineStream>, EngineError> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                window: LiveWindow::new(self.config),
                decoding: false,
                stalled: false,
                finishing: false,
                closed: false,
                failure: None,
            }),
            wake: Condvar::new(),
            send: Mutex::new(()),
            events,
            engine: self.engine.clone(),
            channel,
            cancel: CancelToken::new(),
        });
        let worker = {
            let shared = shared.clone();
            thread::Builder::new()
                .name("ink-live".into())
                .spawn(move || shared.run())
                .map_err(|e| {
                    EngineError::Failed(format!("the live-partials thread did not start: {e}"))
                })?
        };
        Ok(Box::new(Stream {
            shared,
            worker: Some(worker),
        }))
    }
}

struct State {
    window: LiveWindow,
    /// A decode is under way.
    decoding: bool,
    /// The backlog was reported as a stall and has not come down since.
    stalled: bool,
    /// `finish` was called: decode what is left and end.
    finishing: bool,
    /// The stream was dropped: send nothing more.
    closed: bool,
    /// The decode that failed, which ended the stream.
    failure: Option<EngineError>,
}

/// What a stream and its decode thread share.
struct Shared {
    state: Mutex<State>,
    /// Wakes the decode thread: audio for a decode, the finish, or the drop.
    wake: Condvar,
    /// Held while sending, so events leave one at a time and none after the drop.
    send: Mutex<()>,
    events: EventSink<AsrEvent>,
    engine: Arc<dyn OfflineEngine>,
    channel: Channel,
    /// Cancelled when the stream is dropped, for a decode under way.
    cancel: CancelToken,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    /// The decode thread.
    fn run(&self) {
        let mut s = self.lock();
        loop {
            if s.closed || (s.failure.is_some() && s.finishing) {
                return;
            }
            // A failed stream decodes nothing more: it waits for its finish or its drop.
            let job = if s.failure.is_some() {
                None
            } else if s.finishing {
                Some((s.window.take_last_window(), true))
            } else if s.window.wants_decode() {
                Some((Some(s.window.take_window()), false))
            } else {
                None
            };
            let Some((window, last)) = job else {
                s.decoding = false;
                s = self.wake.wait(s).unwrap_or_else(PoisonError::into_inner);
                continue;
            };
            s.decoding = true;
            drop(s);
            // No lock is held while the engine works: push only appends meanwhile.
            let decoded = window.as_ref().map(|w| self.decode(w));
            s = self.lock();
            if s.closed {
                return;
            }
            let dropped = s.window.take_unheard_drops();
            let outputs = match (&window, decoded) {
                (Some(w), Some(Ok(words))) if last => s.window.apply_last(&words, w),
                (Some(w), Some(Ok(words))) => s.window.apply(&words, w),
                (_, Some(Err(e))) => {
                    s.failure = Some(e);
                    Vec::new()
                }
                // The end, with too little audio left to decode.
                _ => s.window.end_without_window(),
            };
            if s.window.backlog() < s.window.config.hop {
                s.stalled = false;
            }
            drop(s);
            self.send_all(outputs, dropped);
            if last {
                return;
            }
            s = self.lock();
        }
    }

    /// One decode of `window`: its words, in samples from its start.
    fn decode(&self, window: &Window) -> Result<Vec<Word>, EngineError> {
        let options = TranscribeOptions {
            channel: self.channel,
            context: None,
            cancel: self.cancel.clone(),
        };
        let transcript = self.engine.transcribe(&window.samples, &options)?;
        Ok(words_of(&transcript, window.samples.len()))
    }

    /// Sends `outputs`, after a report of `dropped` samples let go of unheard, unless the stream
    /// was dropped.
    fn send_all(&self, outputs: Vec<Output>, dropped: usize) {
        if outputs.is_empty() && dropped == 0 {
            return;
        }
        let _sending = lock(&self.send);
        if self.lock().closed {
            return;
        }
        if dropped > 0 {
            (self.events)(AsrEvent::Stalled {
                reason: format!(
                    "a decode fell behind and {:.1} s of audio went unheard; the final pass still \
                     has it",
                    dropped as f64 / RATE as f64
                ),
            });
        }
        for output in outputs {
            (self.events)(match output {
                Output::Partial(text) => AsrEvent::Partial { text },
                Output::Final(text) => AsrEvent::Final(text),
            });
        }
    }
}

/// One side's live stream. See the module docs for its threads.
struct Stream {
    shared: Arc<Shared>,
    /// The decode thread, until `finish` joins it.
    worker: Option<JoinHandle<()>>,
}

impl EngineStream for Stream {
    fn push(&mut self, audio: &[f32]) -> Result<(), EngineError> {
        let stall = {
            let mut s = self.shared.lock();
            if let Some(failure) = &s.failure {
                return Err(failure.clone());
            }
            s.window.append(audio);
            if !s.decoding && s.window.wants_decode() {
                self.shared.wake.notify_one();
            }
            let stall = s.decoding && !s.stalled && s.window.backlog() >= STALL_AFTER;
            s.stalled |= stall;
            stall
        };
        if stall {
            let _sending = lock(&self.shared.send);
            (self.shared.events)(AsrEvent::Stalled {
                reason: "3 s of audio wait for a decode".into(),
            });
        }
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<(), EngineError> {
        self.shared.lock().finishing = true;
        self.shared.wake.notify_all();
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            return Err(EngineError::Failed(
                "the live-partials thread panicked".into(),
            ));
        }
        match self.shared.lock().failure.clone() {
            Some(failure) => Err(failure),
            None => Ok(()),
        }
    }
}

impl Drop for Stream {
    /// Without a finish: nothing more is sent once this returns, and a decode under way is told to
    /// stop. The thread is not waited for; it ends after that decode.
    fn drop(&mut self) {
        if self.worker.take().is_some() {
            {
                let _sending = lock(&self.shared.send);
                self.shared.lock().closed = true;
            }
            self.shared.cancel.cancel();
            self.shared.wake.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    /// A word of a script, in stream samples.
    #[derive(Clone, Debug)]
    struct ScriptWord {
        text: String,
        start: usize,
        end: usize,
    }

    /// Unbroken speech: `count` words of 0.25 s with 0.05 s between them, from `from`.
    fn unbroken(count: usize, from: usize, prefix: &str) -> Vec<ScriptWord> {
        (0..count)
            .map(|i| {
                let start = from + i * 4_800;
                ScriptWord {
                    text: format!("{prefix}{i}"),
                    start,
                    end: start + 4_000,
                }
            })
            .collect()
    }

    /// Stream audio whose every sample is its own index, so a decoder handed a window knows where
    /// it sits in the stream (exact in f32 up to 2^24 samples: 17 minutes).
    fn indexed(from: usize, to: usize) -> Vec<f32> {
        (from..to).map(|i| i as f32).collect()
    }

    /// `window` decoded as the words of `script` fully inside it.
    fn decode(window: &Window, script: &[ScriptWord]) -> Vec<Word> {
        script
            .iter()
            .filter(|w| w.start >= window.start && w.end <= window.end())
            .map(|w| Word {
                text: w.text.clone(),
                start: w.start - window.start,
                end: w.end - window.start,
            })
            .collect()
    }

    /// Feeds `seconds` of stream in blocks of `block`, decoding whenever the scheme wants to, then
    /// ends the stream. Each output comes with the stream sample at which it was sent.
    fn drive(
        script: &[ScriptWord],
        seconds: f64,
        block: usize,
        mut inspect: impl FnMut(&LiveWindow),
    ) -> Vec<(usize, Output)> {
        let mut live = LiveWindow::new(LiveConfig::default());
        let mut sent = Vec::new();
        let total = (seconds * RATE as f64) as usize;
        let mut t = 0;
        while t < total {
            let n = block.min(total - t);
            live.append(&indexed(t, t + n));
            t += n;
            if live.wants_decode() {
                let window = live.take_window();
                let words = decode(&window, script);
                sent.extend(live.apply(&words, &window).into_iter().map(|o| (t, o)));
                inspect(&live);
            }
        }
        let outs = match live.take_last_window() {
            Some(window) => {
                let words = decode(&window, script);
                live.apply_last(&words, &window)
            }
            None => live.end_without_window(),
        };
        sent.extend(outs.into_iter().map(|o| (t, o)));
        sent
    }

    fn finals(sent: &[(usize, Output)]) -> Vec<(usize, TimedText)> {
        sent.iter()
            .filter_map(|(at, o)| match o {
                Output::Final(s) => Some((*at, s.clone())),
                Output::Partial(_) => None,
            })
            .collect()
    }

    fn all_words(finals: &[(usize, TimedText)]) -> Vec<String> {
        finals
            .iter()
            .flat_map(|(_, s)| s.text.split(' ').map(str::to_owned).collect::<Vec<_>>())
            .collect()
    }

    fn texts(script: &[ScriptWord]) -> Vec<String> {
        script.iter().map(|w| w.text.clone()).collect()
    }

    #[test]
    fn a_pause_after_the_last_word_settles_the_utterance() {
        let script = unbroken(5, 8_000, "w"); // 0.5 s to about 1.95 s
        let sent = drive(&script, 4.0, 1_600, |_| {});
        let settled = finals(&sent);
        assert_eq!(settled.len(), 1);
        assert_eq!(settled[0].1.text, "w0 w1 w2 w3 w4");
        assert_eq!(settled[0].1.start_ms, 500);
        assert_eq!(settled[0].1.end_ms, (script[4].end / 16) as u64);
        // Settled once the pause is heard (0.8 s after the last word), not at the stream's end.
        assert!(settled[0].0 < script[4].end + 12_800 + 8_000 + 1_600);
        assert_eq!(sent.last().unwrap().1, Output::Partial(String::new()));
    }

    /// Unbroken speech is settled by length, without waiting for a pause.
    #[test]
    fn unbroken_speech_is_settled_without_waiting_for_a_pause() {
        let script = unbroken(100, 8_000, "w"); // 30 s without a pause
        let settled = finals(&drive(&script, 31.0, 1_600, |_| {}));
        let before_the_end = settled
            .iter()
            .filter(|(at, _)| *at < script.last().unwrap().end)
            .count();
        assert!(before_the_end >= 2, "settled while still speaking");
        // The first settles once the utterance reaches 12 s, give or take a hop.
        assert!(settled[0].0 <= 8_000 + 192_000 + 8_000);
        for (_, s) in &settled {
            assert!(s.end_ms - s.start_ms <= 12_000, "{s:?}");
        }
        assert_eq!(all_words(&settled), texts(&script), "every word, once");
    }

    /// The bound is audio since the utterance began: how the audio is pushed cannot move it.
    #[test]
    fn how_audio_is_pushed_cannot_postpone_the_bound() {
        let script = unbroken(100, 8_000, "w");
        let mut first = Vec::new();
        for block in [160, 5_120, 16_000] {
            let settled = finals(&drive(&script, 31.0, block, |_| {}));
            assert!(!settled.is_empty());
            first.push(settled[0].0);
            assert!(
                settled[0].0 <= 8_000 + 192_000 + 8_000 + block,
                "block {block}"
            );
        }
        let spread = first.iter().max().unwrap() - first.iter().min().unwrap();
        assert!(spread <= 16_000, "{first:?}");
    }

    /// A final carries only its own words, and a partial only words not settled yet.
    #[test]
    fn finals_carry_only_their_own_words_and_partials_only_unsettled_ones() {
        let script: Vec<ScriptWord> = unbroken(6, 8_000, "a")
            .into_iter()
            .chain(unbroken(4, 80_000, "b"))
            .chain(unbroken(5, 144_000, "c"))
            .collect();
        let sent = drive(&script, 14.0, 1_600, |_| {});
        let settled: Vec<String> = finals(&sent).into_iter().map(|(_, s)| s.text).collect();
        assert_eq!(
            settled,
            ["a0 a1 a2 a3 a4 a5", "b0 b1 b2 b3", "c0 c1 c2 c3 c4"]
        );
        let utterances = ["a", "b", "c", "none after the last"];
        let mut settled_so_far = 0;
        for (_, output) in &sent {
            match output {
                Output::Final(_) => settled_so_far += 1,
                Output::Partial(p) => {
                    let utterance = utterances[settled_so_far];
                    assert!(
                        p.split_whitespace().all(|w| w.starts_with(utterance)),
                        "{p} during {utterance}"
                    );
                }
            }
        }
    }

    /// Final times come from sample counts: an utterance 20 s in is placed at 20 s.
    #[test]
    fn final_times_are_counted_in_samples() {
        let script = unbroken(3, 320_000, "w"); // after 20 s of silence
        let settled = finals(&drive(&script, 24.0, 1_600, |_| {}));
        assert_eq!(settled.len(), 1);
        assert_eq!(settled[0].1.start_ms, 20_000);
        assert_eq!(settled[0].1.end_ms, (script[2].end / 16) as u64);
    }

    #[test]
    fn silence_keeps_the_window_short() {
        let mut longest = 0;
        drive(&[], 30.0, 1_600, |live| {
            longest = longest.max(live.buffer.len())
        });
        let config = LiveConfig::default();
        assert!(longest <= config.keep_silence + config.hop, "{longest}");
    }

    /// A decoder far behind must not make memory hold the session (architecture rule 3).
    #[test]
    fn a_buffer_past_its_cap_lets_the_oldest_audio_go() {
        let mut live = LiveWindow::new(LiveConfig::default());
        let window = live.take_window();
        for i in 0..120 {
            // two minutes, none of it decoded
            live.append(&indexed(i * RATE, (i + 1) * RATE));
        }
        assert_eq!(live.buffer.len(), live.config.max_buffer);
        assert_eq!(live.dropped_unheard, 120 * RATE - live.config.max_buffer);
        assert_eq!(live.apply(&[], &window), [], "a stale decode is ignored");
        assert_eq!(live.buffer.len(), live.config.max_buffer);
    }

    /// Audio let go of unheard is reported once per stretch, however many pushes it took.
    #[test]
    fn audio_dropped_unheard_is_reported_once_per_stretch() {
        let mut live = LiveWindow::new(LiveConfig::default());
        live.take_window();
        assert_eq!(live.take_unheard_drops(), 0);
        for i in 0..60 {
            live.append(&indexed(i * RATE, (i + 1) * RATE));
        }
        assert_eq!(
            live.take_unheard_drops(),
            60 * RATE - live.config.max_buffer
        );
        assert_eq!(live.take_unheard_drops(), 0, "reported once");
        live.append(&indexed(60 * RATE, 61 * RATE));
        assert_eq!(
            live.take_unheard_drops(),
            RATE,
            "a later stretch is its own report"
        );
        assert_eq!(live.dropped_unheard, 61 * RATE - live.config.max_buffer);
    }

    /// A word ending in the last 0.16 s of the decoded audio is not shown yet, and appears once
    /// audio after it has been decoded. Finals keep every word.
    #[test]
    fn the_partial_hides_words_ending_in_the_newest_audio() {
        assert_eq!(LiveConfig::default().hide_newest, 2_560, "0.16 s");
        let mut live = LiveWindow::new(LiveConfig::default());
        live.append(&indexed(0, 16_000));
        let script = [
            ScriptWord {
                text: "early".into(),
                start: 2_000,
                end: 6_000,
            },
            ScriptWord {
                text: "late".into(),
                start: 10_000,
                end: 16_000 - 1_600,
            },
        ];
        let first = live.take_window();
        let words = decode(&first, &script);
        assert_eq!(
            live.apply(&words, &first),
            [Output::Partial("early".into())]
        );
        live.append(&indexed(16_000, 24_000));
        let second = live.take_window();
        let words = decode(&second, &script);
        assert_eq!(
            live.apply(&words, &second),
            [Output::Partial("early late".into())]
        );
        let script = unbroken(10, 8_000, "w");
        assert_eq!(
            all_words(&finals(&drive(&script, 5.0, 1_600, |_| {}))),
            texts(&script)
        );
    }

    #[test]
    fn the_streams_end_settles_what_is_left() {
        let script = unbroken(10, 8_000, "w");
        // The stream ends 0.1 s after the last word: no pause heard.
        let seconds = (script.last().unwrap().end + 1_600) as f64 / RATE as f64;
        let sent = drive(&script, seconds, 1_600, |_| {});
        assert_eq!(all_words(&finals(&sent)), texts(&script));
        assert_eq!(sent.last().unwrap().1, Output::Partial(String::new()));
    }

    #[test]
    fn partials_change_only_when_the_words_do() {
        let sent = drive(&unbroken(20, 8_000, "w"), 8.0, 1_600, |_| {});
        let partials: Vec<&str> = sent
            .iter()
            .filter_map(|(_, o)| match o {
                Output::Partial(p) => Some(p.as_str()),
                Output::Final(_) => None,
            })
            .collect();
        assert!(partials.len() > 5);
        for pair in partials.windows(2) {
            assert_ne!(pair[0], pair[1], "a partial is sent only when it changes");
        }
    }

    // --- The stream, with an offline engine and its thread. -----------------------------------

    /// An offline engine that decodes `indexed` audio as the words of its script fully inside it,
    /// one segment per word in ms, as the sherpa-onnx adapter returns them. It can be held at the
    /// start of a decode, and made to fail.
    struct Scripted {
        script: Vec<ScriptWord>,
        calls: AtomicUsize,
        hold: Mutex<bool>,
        released: Condvar,
        fail: bool,
    }

    impl Scripted {
        fn with(script: Vec<ScriptWord>, hold: bool, fail: bool) -> Arc<Self> {
            Arc::new(Self {
                script,
                calls: AtomicUsize::new(0),
                hold: Mutex::new(hold),
                released: Condvar::new(),
                fail,
            })
        }

        fn new(script: Vec<ScriptWord>) -> Arc<Self> {
            Self::with(script, false, false)
        }

        fn holding(script: Vec<ScriptWord>) -> Arc<Self> {
            Self::with(script, true, false)
        }

        fn release(&self) {
            *lock(&self.hold) = false;
            self.released.notify_all();
        }
    }

    impl OfflineEngine for Scripted {
        fn info(&self) -> EngineInfo {
            info()
        }

        fn transcribe(
            &self,
            audio: &[f32],
            options: &TranscribeOptions,
        ) -> Result<Transcript, EngineError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut held = lock(&self.hold);
            while *held {
                held = self
                    .released
                    .wait(held)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            drop(held);
            if self.fail {
                return Err(EngineError::Failed("scripted failure".into()));
            }
            if options.cancel.is_cancelled() {
                return Err(EngineError::Cancelled);
            }
            let window = Window {
                samples: audio.to_vec(),
                start: audio.first().map_or(0, |s| *s as usize),
            };
            Ok(Transcript {
                segments: decode(&window, &self.script)
                    .into_iter()
                    .map(|w| TimedText {
                        start_ms: (w.start / PER_MS) as u64,
                        end_ms: (w.end / PER_MS) as u64,
                        text: w.text,
                    })
                    .collect(),
            })
        }
    }

    fn info() -> EngineInfo {
        EngineInfo {
            id: "scripted".into(),
            jobs: vec![ink_core::Job::LivePartials],
            licence: "MIT".into(),
        }
    }

    /// A sink that keeps every event.
    fn sink() -> (EventSink<AsrEvent>, Arc<Mutex<Vec<AsrEvent>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let kept = events.clone();
        (Arc::new(move |e| lock(&kept).push(e)), events)
    }

    /// Waits up to 10 s for `done`.
    fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let until = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < until, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn open(engine: &Arc<Scripted>) -> (Box<dyn EngineStream>, Arc<Mutex<Vec<AsrEvent>>>) {
        let live = TrailingWindow::new(engine.clone(), info());
        let (events, kept) = sink();
        (live.open_stream(Channel::Mic, events).unwrap(), kept)
    }

    #[test]
    fn a_stream_sends_partials_and_finals_and_its_finish_sends_the_rest() {
        let script = unbroken(5, 8_000, "w");
        let engine = Scripted::new(script.clone());
        let (mut stream, kept) = open(&engine);
        assert_eq!(TrailingWindow::new(engine.clone(), info()).info(), info());
        // 1.2 s, two words in, in one push so the first window is all of it: the first word shows
        // as a partial (the second ends in the newest 0.16 s). Pushed in blocks, the thread could
        // take its window early, hide the only word it has, and wait for a hop that never comes.
        stream.push(&indexed(0, 19_200)).unwrap();
        wait_for("a partial", || {
            lock(&kept)
                .iter()
                .any(|e| matches!(e, AsrEvent::Partial { text } if !text.is_empty()))
        });
        // To 4 s: the rest of the words, then a pause long enough to settle them.
        for t in (19_200..4 * RATE).step_by(1_600) {
            stream.push(&indexed(t, t + 1_600)).unwrap();
        }
        wait_for("the final", || {
            lock(&kept).iter().any(|e| matches!(e, AsrEvent::Final(_)))
        });
        stream.push(&indexed(4 * RATE, 5 * RATE)).unwrap();
        stream.finish().unwrap();
        let events = lock(&kept).clone();
        let finals: Vec<&TimedText> = events
            .iter()
            .filter_map(|e| match e {
                AsrEvent::Final(t) => Some(t),
                _ => None,
            })
            .collect();
        assert_eq!(finals.len(), 1, "{events:?}");
        assert_eq!(finals[0].text, "w0 w1 w2 w3 w4");
        assert_eq!(finals[0].start_ms, 500);
        assert_eq!(finals[0].end_ms, (script[4].end / 16) as u64);
        assert_eq!(
            events.last(),
            Some(&AsrEvent::Partial {
                text: String::new()
            }),
            "the partial is cleared"
        );
        assert!(engine.calls.load(Ordering::SeqCst) >= 2);
    }

    #[test]
    fn the_finish_settles_words_the_stream_ended_on() {
        let script = unbroken(10, 8_000, "w");
        let engine = Scripted::new(script.clone());
        let (mut stream, kept) = open(&engine);
        let end = script.last().unwrap().end + 1_600;
        for t in (0..end).step_by(1_600) {
            stream.push(&indexed(t, (t + 1_600).min(end))).unwrap();
        }
        stream.finish().unwrap();
        let words: Vec<String> = lock(&kept)
            .iter()
            .filter_map(|e| match e {
                AsrEvent::Final(t) => Some(t.text.clone()),
                _ => None,
            })
            .flat_map(|t| t.split(' ').map(str::to_owned).collect::<Vec<_>>())
            .collect();
        assert_eq!(
            words,
            texts(&script),
            "every word, once, by the time finish returns"
        );
    }

    #[test]
    fn a_slow_engine_is_a_stall_reported_once_and_push_never_waits() {
        let engine = Scripted::holding(unbroken(5, 8_000, "w"));
        let (mut stream, kept) = open(&engine);
        stream.push(&indexed(0, RATE)).unwrap();
        wait_for("the first decode", || {
            engine.calls.load(Ordering::SeqCst) == 1
        });
        // The decode is held: 5 s more audio arrives, and every push returns at once.
        let started = Instant::now();
        for t in (RATE..6 * RATE).step_by(1_600) {
            stream.push(&indexed(t, t + 1_600)).unwrap();
        }
        assert!(started.elapsed() < Duration::from_secs(2), "push waited");
        let stalls = lock(&kept)
            .iter()
            .filter(|e| matches!(e, AsrEvent::Stalled { .. }))
            .count();
        assert_eq!(stalls, 1, "{:?}", lock(&kept));
        engine.release();
        stream.finish().unwrap();
    }

    #[test]
    fn a_failed_decode_ends_the_stream_with_its_error() {
        let engine = Scripted::with(unbroken(5, 8_000, "w"), false, true);
        let (mut stream, kept) = open(&engine);
        stream.push(&indexed(0, RATE)).unwrap();
        wait_for("the failed decode", || {
            engine.calls.load(Ordering::SeqCst) == 1
        });
        let failed = EngineError::Failed("scripted failure".into());
        wait_for("the failure to reach push", || {
            stream.push(&indexed(0, 160)) == Err(failed.clone())
        });
        assert_eq!(stream.finish(), Err(failed));
        assert!(lock(&kept).is_empty(), "a failed decode sends no words");
    }

    #[test]
    fn a_dropped_stream_sends_nothing_more_and_its_thread_ends() {
        let engine = Scripted::holding(unbroken(5, 8_000, "w"));
        let (mut stream, kept) = open(&engine);
        stream.push(&indexed(0, 2 * RATE)).unwrap();
        wait_for("the decode", || engine.calls.load(Ordering::SeqCst) == 1);
        drop(stream);
        engine.release();
        // The thread holds the engine until it has seen the drop and ended.
        wait_for("the thread to end", || Arc::strong_count(&engine) == 1);
        assert!(lock(&kept).is_empty(), "{:?}", lock(&kept));
    }

    #[test]
    fn an_engine_that_places_no_words_still_settles_by_length() {
        // One segment for the whole window: no pause is ever heard, but 12 s is.
        let words = words_of(
            &Transcript {
                segments: vec![TimedText {
                    start_ms: 0,
                    end_ms: 20_000,
                    text: " all of it ".into(),
                }],
            },
            16_000,
        );
        assert_eq!(
            words,
            [Word {
                text: "all of it".into(),
                start: 0,
                end: 16_000
            }],
            "kept inside the window"
        );
    }
}
