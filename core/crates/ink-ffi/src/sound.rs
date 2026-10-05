//! Settings > Sound in the core: the devices and the user's choice, their changes, and the mic
//! test. The choice itself is resolved by [`devices`](crate::devices).
//!
//! | Command | Answer |
//! |---|---|
//! | `audio.devices` | `audio.devices`: inputs, outputs (where there is an output picker), the choice, Automatic's pick and the mic in use now, and why |
//! | `audio.test {seconds?}` | `audio.test_started`, then `audio.test_level` about ten times a second, then `audio.tested` (from the sound thread) |
//! | `audio.test_stop` | the running test ends: `audio.tested` with `ended` `stopped` |
//! | `setting.set` of `audio.input`, `audio.output` | `setting.value` ([`queries`](crate::queries)); a device must be connected now, and its name and transport are remembered beside it |
//!
//! Each answer echoes the command's `id` as `ref`; a failure is `command.failed` with that id.
//!
//! **One thread, `ink-sound`**, owns the device watch and the test. It blocks while there is
//! nothing to do (architecture rule 9: no polling timers). The platform's notifications
//! ([`CaptureControl::watch_devices`]) only enqueue; a burst is read once it goes quiet
//! ([`Coalescer`]), and then the shell hears `audio.devices_changed` (the same fields as
//! `audio.devices`) and dictation's idle mic is looked at again ([`Sound::mark_stale`]: let go of
//! when the mic it would open now is another). However hard the OS calls, at most one change waits
//! in the thread's queue. A platform that cannot watch yet is said once in the log; the devices
//! are then read when a mic opens or a screen asks. Nothing here takes the voice slot's lock, which
//! dictation's start holds while it joins threads: the capture and the stale flag live in
//! [`Sound`].
//!
//! **The test** opens the chosen mic (the same pick as dictation and meetings) for up to
//! [`TEST_MAX`], and reports its level from the ink's band analyzer on a meter scale
//! ([`meter`]): no gain stage, so a quiet mic shows as quiet. It is refused while a meeting
//! records (the screen says why: `code` `meeting_recording`), and a meeting that starts ends it
//! within [`LEVEL_INTERVAL`]. Nothing it hears is kept or sent anywhere.
//!
//! **A chosen mic that is not connected** is said once per spell (`audio.input_fallback`), when a
//! mic opens on Automatic in its place: a take, a meeting, a meeting's mic that went, or a test.
//! The spell ends when the chosen mic is seen again, or the choice changes.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ink_audio::{BandAnalyzer, Bands, capture_ring};
use ink_core::{AudioSource, CaptureControl, DeviceChange, DeviceId, EventSink, PlatformError};
use ink_pipeline::mic::MicPath;
use serde_json::Value;

use crate::capture::{MicInfo, transport_name};
use crate::devices::{
    self, AUTO, Coalescer, DEFAULT, INPUT_DEVICE_KEY, INPUT_KEY, InputChoice, MicReason,
    OUTPUT_DEVICE_KEY, OutputChoice, Wanted,
};
use crate::events::{self, event};
use crate::hub::Events;
use crate::meeting::PUMP_INTERVAL;
use crate::runtime::{Runs, Shared, lock};

/// The longest mic test, and what `audio.test` runs without `seconds`.
pub const TEST_MAX: Duration = Duration::from_secs(15);

/// How often a test reports its level.
pub const LEVEL_INTERVAL: Duration = Duration::from_millis(100);

/// The quietest level the meter shows above zero, in dBFS: the meter reads 0 here and below, and
/// 1 at full scale, linear in dB between.
pub const METER_FLOOR_DB: f32 = -60.0;

/// The level a test counts as hearing something (`heard`), in dBFS: quiet speech a little way
/// from the mic is well above it, a silent room or a muted mic below.
pub const HEARD_DB: f32 = -50.0;

/// One hop's bands as a level in dBFS: their combined RMS (the bands do not overlap).
pub fn level_db(b: Bands) -> f32 {
    let rms = (b.low * b.low + b.mid * b.mid + b.high * b.high).sqrt();
    if rms > 0.0 {
        20.0 * rms.log10()
    } else {
        f32::NEG_INFINITY
    }
}

/// A level in dBFS on the meter's 0-1 scale ([`METER_FLOOR_DB`] to full scale).
pub fn meter(db: f32) -> f32 {
    ((db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0)
}

/// What [`Shared`] holds for Sound: the capture, the fallback spell, dictation's stale flag and
/// the sound thread's mailbox.
#[derive(Default)]
pub struct Sound {
    /// The shell platform's capture (dictation's mic), once it is set.
    capture: Mutex<Option<Arc<dyn CaptureControl>>>,
    /// The chosen mic whose absence has been said (`audio.input_fallback`), while it lasts.
    spell: Mutex<Option<DeviceId>>,
    /// The devices or the mic choice changed since dictation's open mic was picked.
    stale: AtomicBool,
    mailbox: OnceLock<Mutex<Sender<Msg>>>,
}

impl Sound {
    /// The capture Sound lists, watches and tests with: the shell platform's, which dictation
    /// opens its mic through. `None` before it is set, or on a build without one.
    pub(crate) fn capture(&self) -> Option<Arc<dyn CaptureControl>> {
        lock(&self.capture).clone()
    }

    fn set_capture(&self, capture: Arc<dyn CaptureControl>) {
        *lock(&self.capture) = Some(capture);
    }

    /// **Any thread.** A device came or went, a default changed, or the mic choice did:
    /// dictation's idle mic is looked at again ([`crate::voice`]: let go of when the mic it would
    /// open now is another; a take finishes on its device first).
    pub(crate) fn mark_stale(&self) {
        self.stale.store(true, Ordering::Release);
    }

    /// **Dictation's mic thread.** Whether [`mark_stale`](Self::mark_stale) was called since the
    /// last time; clears it.
    pub(crate) fn take_stale(&self) -> bool {
        self.stale.swap(false, Ordering::AcqRel)
    }

    /// **Any worker.** A mic opened as `mic`: a stand-in for a chosen mic that is not connected is
    /// said once per spell; the chosen mic (or Automatic chosen) ends the spell.
    pub fn opened_on(&self, events: &Events, mic: &MicInfo) {
        let Some(wanted) = &mic.wanted else {
            *lock(&self.spell) = None;
            return;
        };
        let new_spell = lock(&self.spell).replace(wanted.id.clone()).as_ref() != Some(&wanted.id);
        if new_spell {
            log::info!(
                "the chosen mic is not connected; recording with {} until it is",
                mic.name
            );
            events.emit(event(
                "audio.input_fallback",
                &[
                    ("wanted", Some(wanted.json())),
                    ("mic_name", Some(mic.name.as_str().into())),
                    ("mic_transport", Some(transport_name(mic.transport).into())),
                ],
            ));
        }
    }

    /// The chosen mic was seen connected, or the choice changed: the spell is over.
    fn spell_over(&self) {
        *lock(&self.spell) = None;
    }

    fn send(&self, msg: Msg) -> Result<(), String> {
        let Some(tx) = self.mailbox.get() else {
            return Err("the sound thread has not started".into());
        };
        lock(tx)
            .send(msg)
            .map_err(|_| "the sound thread has stopped".to_owned())
    }
}

/// A Sound command, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SoundQuery {
    /// `audio.devices`.
    Devices,
    /// `audio.test`, for this long.
    Test(Duration),
    /// `audio.test_stop`.
    TestStop,
}

/// Reads `v` as a Sound command: `None` when `name` is not one. Unknown fields are refused.
pub fn parse(name: &str, v: &Value) -> Option<Result<SoundQuery, String>> {
    let allowed: &[&str] = match name {
        "audio.devices" | "audio.test_stop" => &[],
        "audio.test" => &["seconds"],
        _ => return None,
    };
    Some(parse_known(name, allowed, v))
}

fn parse_known(name: &str, allowed: &[&str], v: &Value) -> Result<SoundQuery, String> {
    let obj = v.as_object().ok_or("command: not an object")?;
    if let Some(k) = obj
        .keys()
        .find(|k| !["cmd", "id"].contains(&k.as_str()) && !allowed.contains(&k.as_str()))
    {
        return Err(format!("{name}: unknown field \"{k}\""));
    }
    Ok(match name {
        "audio.devices" => SoundQuery::Devices,
        "audio.test_stop" => SoundQuery::TestStop,
        _ => SoundQuery::Test(match obj.get("seconds") {
            None => TEST_MAX,
            Some(n) => n
                .as_u64()
                .filter(|n| (1..=TEST_MAX.as_secs()).contains(n))
                .map(Duration::from_secs)
                .ok_or_else(|| {
                    format!(
                        "{name}: \"seconds\" is a whole number from 1 to {}",
                        TEST_MAX.as_secs()
                    )
                })?,
        }),
    })
}

/// **Queries thread.** Answers a Sound command: its answer, or `None` when the sound thread
/// answers (the test).
pub fn answer(
    shared: &Shared,
    query: SoundQuery,
    reference: Option<&str>,
) -> Result<Option<Value>, String> {
    match query {
        SoundQuery::Devices => {
            let capture = shared.sound.capture().ok_or(NO_DEVICES)?;
            devices_event(shared, capture.as_ref(), "audio.devices", reference).map(Some)
        }
        SoundQuery::Test(length) => shared
            .sound
            .send(Msg::Test {
                id: reference.map(str::to_owned),
                length,
            })
            .map(|()| None),
        SoundQuery::TestStop => shared
            .sound
            .send(Msg::StopTest {
                id: reference.map(str::to_owned),
            })
            .map(|()| None),
    }
}

/// Why there is nothing to list or test.
const NO_DEVICES: &str = "this build has no audio devices on this platform";

/// **Queries thread.** `setting.set` of [`INPUT_KEY`] or [`OUTPUT_KEY`]: the keyword as it is; a
/// device only when it is connected now, written with what is remembered of it in one write.
pub fn set_choice(shared: &Shared, key: &str, value: &str) -> Result<(), String> {
    let (keyword, device_key) = if key == INPUT_KEY {
        (AUTO, INPUT_DEVICE_KEY)
    } else {
        (DEFAULT, OUTPUT_DEVICE_KEY)
    };
    let store = shared.store.as_ref();
    if value == keyword {
        return store.set_setting(key, value).map_err(|e| e.to_string());
    }
    let capture = shared.sound.capture().ok_or(NO_DEVICES)?;
    let listed = if key == INPUT_KEY {
        capture.input_devices()
    } else {
        capture.output_devices()
    };
    let listed = listed.map_err(|e| match e {
        PlatformError::Unsupported(_) => {
            format!("setting.set: \"{key}\" takes only {keyword} on this platform")
        }
        e => format!("setting.set: the devices could not be read: {e}"),
    })?;
    let device = listed.iter().find(|d| d.id.0 == value).ok_or_else(|| {
        format!("setting.set: no device with that id is connected; \"{key}\" keeps its value")
    })?;
    let remembered = Wanted::of(device).json().to_string();
    store
        .set_settings(&[(device_key, &remembered), (key, value)])
        .map_err(|e| e.to_string())
}

/// **Queries thread.** A choice was written: the spell of a mic that left is over, dictation's idle
/// mic is looked at again, and the screens hear the devices with the new choice.
pub fn choice_changed(shared: &Shared, key: &str) {
    if key == INPUT_KEY {
        shared.sound.spell_over();
        shared.sound.mark_stale();
    }
    if shared.sound.send(Msg::Changed(None)).is_err() {
        log::warn!("the sound thread has stopped; the devices are not said again");
    }
}

/// **Worker.** `audio.devices` (or `audio.devices_changed`, `ty`): the lists, the choice, what
/// Automatic picks and what records now. A chosen mic seen connected here ends its fallback spell,
/// whoever asked.
fn devices_event(
    shared: &Shared,
    capture: &dyn CaptureControl,
    ty: &str,
    reference: Option<&str>,
) -> Result<Value, String> {
    let read = |e: PlatformError| format!("the devices could not be read: {e}");
    let inputs = capture.input_devices().map_err(read)?;
    let outputs = match capture.output_devices() {
        Ok(outputs) => Some(outputs),
        Err(PlatformError::Unsupported(_)) => None,
        Err(e) => return Err(read(e)),
    };
    // Without Automatic's pick the lists still go out: a screen with devices beats none.
    let automatic = capture.automatic_input().unwrap_or_else(|e| {
        log::warn!("{ty}: Automatic's mic could not be read: {e}");
        None
    });
    let store = shared.store.as_ref();
    let choice = devices::input_choice(store);
    let using = devices::resolve_input(&inputs, &choice, automatic.clone());
    if using
        .as_ref()
        .is_some_and(|p| p.reason == MicReason::Chosen)
    {
        shared.sound.spell_over();
    }
    let list =
        |l: &[ink_core::DeviceInfo]| Value::Array(l.iter().map(devices::device_json).collect());
    let mut fields: Vec<(&str, Option<Value>)> = vec![
        ("inputs", Some(list(&inputs))),
        (
            "input",
            Some(match &choice {
                InputChoice::Auto => AUTO.into(),
                InputChoice::Device(w) => w.id.0.as_str().into(),
            }),
        ),
        (
            "wanted",
            match &choice {
                InputChoice::Device(w) => Some(w.json()),
                InputChoice::Auto => None,
            },
        ),
        (
            "automatic",
            automatic.map(|a| {
                devices::Picked {
                    device: a.device,
                    reason: MicReason::Auto(a.reason),
                    wanted: None,
                }
                .json()
            }),
        ),
        ("using", using.map(|p| p.json())),
        ("ref", reference.map(Into::into)),
    ];
    if let Some(outputs) = outputs {
        let choice = devices::output_choice(store);
        fields.push(("outputs", Some(list(&outputs))));
        fields.push((
            "output",
            Some(match &choice {
                OutputChoice::Default => DEFAULT.into(),
                OutputChoice::Device(w) => w.id.0.as_str().into(),
            }),
        ));
        fields.push((
            "output_wanted",
            match &choice {
                OutputChoice::Device(w) => Some(w.json()),
                OutputChoice::Default => None,
            },
        ));
        fields.push((
            "output_using",
            devices::resolve_output(&outputs, &choice).map(|p| p.json()),
        ));
    }
    Ok(event(ty, &fields))
}

/// Messages for `ink-sound`.
pub(crate) enum Msg {
    /// The platform to watch and test with (once the shell's platform is set).
    Watch(Arc<dyn CaptureControl>),
    /// A device notification, or (`None`) a choice written.
    Changed(Option<DeviceChange>),
    /// `audio.test`.
    Test {
        id: Option<String>,
        length: Duration,
    },
    /// `audio.test_stop`.
    StopTest { id: Option<String> },
    /// End the thread.
    Quit,
}

/// The sound thread (the module docs).
pub struct SoundThread {
    tx: Sender<Msg>,
    thread: JoinHandle<()>,
}

impl SoundThread {
    /// Starts `ink-sound` and gives [`Shared`] its mailbox. `runs` tells it whether a meeting
    /// records.
    pub fn start(shared: Arc<Shared>, runs: Arc<Mutex<Runs>>) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel::<Msg>();
        let thread = {
            let (shared, tx) = (shared.clone(), tx.clone());
            thread::Builder::new()
                .name("ink-sound".into())
                .spawn(move || {
                    Worker {
                        shared: &shared,
                        runs: &runs,
                        tx,
                        pending: Arc::default(),
                        capture: None,
                        coalescer: Coalescer::default(),
                        test: None,
                    }
                    .run(&rx);
                })?
        };
        let _ = shared.sound.mailbox.set(Mutex::new(tx.clone()));
        Ok(Self { tx, thread })
    }

    /// Gives Sound the platform's capture: listed and tested from now, and watched.
    pub fn watch(&self, shared: &Shared, capture: Arc<dyn CaptureControl>) {
        shared.sound.set_capture(capture.clone());
        let _ = self.tx.send(Msg::Watch(capture));
    }

    /// Ends a test in progress, stops watching, and ends the thread.
    pub fn stop(self) {
        let _ = self.tx.send(Msg::Quit);
        if self.thread.join().is_err() {
            log::error!("the sound thread panicked");
        }
    }
}

/// **Callback thread.** The platform's device callback: it only enqueues, and at most one change
/// waits in the queue (`pending` is set until the thread takes it), however hard the OS calls.
fn change_sink(tx: Sender<Msg>, pending: Arc<AtomicBool>) -> EventSink<DeviceChange> {
    let tx = Mutex::new(tx);
    Arc::new(move |change: DeviceChange| {
        if !pending.swap(true, Ordering::AcqRel) {
            // Refused only once the thread has gone: nothing is left to tell.
            let _ = lock(&tx).send(Msg::Changed(Some(change)));
        }
    })
}

/// A mic test in progress.
struct Test {
    id: Option<String>,
    source: Box<dyn AudioSource>,
    ring: ink_audio::CaptureConsumer,
    path: MicPath,
    analyzer: BandAnalyzer,
    until: Instant,
    next_level: Instant,
    /// The loudest hop since the last level, in dBFS.
    window_db: f32,
    /// The loudest hop of the test, in dBFS.
    peak_db: f32,
}

/// How a test ended (`AudioTestEnd`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ended {
    Done,
    Stopped,
    Meeting,
    Failed,
}

impl Ended {
    fn word(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Stopped => "stopped",
            Self::Meeting => "meeting",
            Self::Failed => "failed",
        }
    }
}

struct Worker<'a> {
    shared: &'a Shared,
    runs: &'a Mutex<Runs>,
    /// For the platform's callback.
    tx: Sender<Msg>,
    /// A change from the callback waits in the queue: the callback sends no other until it is
    /// taken (the read it asks for covers them all).
    pending: Arc<AtomicBool>,
    capture: Option<Arc<dyn CaptureControl>>,
    coalescer: Coalescer,
    test: Option<Test>,
}

impl Worker<'_> {
    fn run(mut self, rx: &Receiver<Msg>) {
        loop {
            // Blocks while there is nothing to do; wakes for a test's pump or a burst's deadline.
            let msg = match self.wait() {
                None => match rx.recv() {
                    Ok(msg) => Some(msg),
                    Err(_) => break,
                },
                Some(wait) => match rx.recv_timeout(wait) {
                    Ok(msg) => Some(msg),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                },
            };
            match msg {
                Some(Msg::Quit) => break,
                Some(msg) => self.handle(msg),
                None => {}
            }
            if self.test.is_some() {
                self.pump_test();
            }
            if self.coalescer.take_due(self.shared.clock.now_ns()) {
                self.devices_changed();
            }
        }
        if self.test.is_some() {
            self.end_test(Ended::Stopped, None);
        }
        if let Some(capture) = &self.capture {
            capture.unwatch_devices();
        }
    }

    /// How long to wait for a message: a test's pump interval, a pending burst's deadline, or
    /// `None` (block: nothing to do).
    fn wait(&self) -> Option<Duration> {
        if self.test.is_some() {
            return Some(PUMP_INTERVAL);
        }
        self.coalescer.deadline_ns().map(|d| {
            Duration::from_nanos(d.saturating_sub(self.shared.clock.now_ns()))
                // A mock clock that stands still is looked at again, not waited on forever.
                .max(Duration::from_millis(1))
        })
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Watch(capture) => {
                // The old watcher first: the same capture given again would otherwise lose the
                // new callback to the old one's unwatch.
                if let Some(old) = self.capture.take() {
                    old.unwatch_devices();
                }
                let sink = change_sink(self.tx.clone(), Arc::clone(&self.pending));
                match capture.watch_devices(sink) {
                    Ok(()) => log::info!("watching the audio devices"),
                    Err(PlatformError::Unsupported(what)) => log::info!(
                        "the platform has no {what} yet; the devices are read when a mic opens"
                    ),
                    Err(e) => log::warn!("the audio devices cannot be watched: {e}"),
                }
                self.capture = Some(capture);
            }
            Msg::Changed(change) => {
                if let Some(change) = change {
                    // Taken: the callback may send the next one.
                    self.pending.store(false, Ordering::Release);
                    log::info!("audio devices: {change:?}");
                }
                self.coalescer.changed(self.shared.clock.now_ns());
            }
            Msg::Test { id, length } => self.start_test(id, length),
            Msg::StopTest { id } => {
                if self.test.is_some() {
                    self.end_test(Ended::Stopped, None);
                } else {
                    self.failed(
                        "audio.test_stop",
                        id.as_deref(),
                        "no mic test is running",
                        None,
                    );
                }
            }
            Msg::Quit => {}
        }
    }

    fn failed(&self, command: &str, id: Option<&str>, message: &str, code: Option<&str>) {
        log::warn!("command {command} failed: {message}");
        self.shared
            .events
            .emit(events::command_failed_coded(command, id, message, code));
    }

    /// A burst of changes has gone quiet: the screens hear the devices, dictation looks again.
    fn devices_changed(&self) {
        let Some(capture) = &self.capture else {
            return;
        };
        match devices_event(self.shared, capture.as_ref(), "audio.devices_changed", None) {
            Ok(e) => self.shared.events.emit(e),
            Err(e) => log::warn!("audio.devices_changed: {e}"),
        }
        self.shared.sound.mark_stale();
    }

    fn meeting_records(&self) -> bool {
        lock(self.runs)
            .meeting
            .as_ref()
            .is_some_and(|m| m.is_capturing())
    }

    fn start_test(&mut self, id: Option<String>, length: Duration) {
        const NAME: &str = "audio.test";
        let reference = id.as_deref();
        if self.test.is_some() {
            return self.failed(NAME, reference, "a mic test is already running", None);
        }
        if self.meeting_records() {
            return self.failed(
                NAME,
                reference,
                "the mic can't be tested while a meeting records",
                Some("meeting_recording"),
            );
        }
        let Some(capture) = self.capture.clone() else {
            return self.failed(NAME, reference, NO_DEVICES, None);
        };
        match open_test(self.shared, capture.as_ref()) {
            Ok((source, ring, path, mic)) => {
                self.shared.sound.opened_on(&self.shared.events, &mic);
                self.shared.events.emit(event(
                    "audio.test_started",
                    &[
                        ("mic_name", Some(mic.name.as_str().into())),
                        ("mic_transport", Some(transport_name(mic.transport).into())),
                        ("mic_reason", Some(mic.reason.into())),
                        ("seconds", Some(length.as_secs().into())),
                        ("ref", reference.map(Into::into)),
                    ],
                ));
                let now = Instant::now();
                self.test = Some(Test {
                    id,
                    source,
                    ring,
                    path,
                    analyzer: BandAnalyzer::new(),
                    until: now + length,
                    next_level: now + LEVEL_INTERVAL,
                    window_db: f32::NEG_INFINITY,
                    peak_db: f32::NEG_INFINITY,
                });
            }
            Err(e) => self.failed(NAME, reference, &e, None),
        }
    }

    /// Drains the test's ring through the band analyzer; reports the level each
    /// [`LEVEL_INTERVAL`]; ends it when its time is up, its mic went, or a meeting records.
    fn pump_test(&mut self) {
        let Some(t) = self.test.as_mut() else {
            return;
        };
        let mut failure = None;
        while let Some(captured) = t.ring.pop() {
            match t.path.push(&captured.block) {
                Ok(out) => {
                    if let Some(b) = t.analyzer.process(out.samples) {
                        let db = level_db(b);
                        t.window_db = t.window_db.max(db);
                        t.peak_db = t.peak_db.max(db);
                    }
                }
                Err(e) => {
                    failure = Some(e.to_string());
                    break;
                }
            }
        }
        if failure.is_none() && t.source.ended() {
            failure = Some(
                t.source
                    .stop()
                    .err()
                    .map_or_else(|| "the microphone stopped".to_owned(), |e| e.to_string()),
            );
        }
        if let Some(why) = failure {
            return self.end_test(Ended::Failed, Some(why));
        }
        let now = Instant::now();
        if now >= t.until {
            return self.end_test(Ended::Done, None);
        }
        if now >= t.next_level {
            t.next_level = now + LEVEL_INTERVAL;
            let level = meter(std::mem::replace(&mut t.window_db, f32::NEG_INFINITY));
            let reference = t.id.clone();
            self.shared.events.emit(event(
                "audio.test_level",
                &[
                    ("level", Some(f64::from(level).into())),
                    ("ref", reference.map(Into::into)),
                ],
            ));
            if self.meeting_records() {
                self.end_test(Ended::Meeting, None);
            }
        }
    }

    fn end_test(&mut self, ended: Ended, message: Option<String>) {
        let Some(mut t) = self.test.take() else {
            return;
        };
        if let Err(e) = t.source.stop()
            && ended != Ended::Failed
        {
            log::warn!("audio.test: the mic did not stop cleanly: {e}");
        }
        if let Some(why) = &message {
            log::warn!("audio.test: {why}");
        }
        self.shared.events.emit(event(
            "audio.tested",
            &[
                ("ended", Some(ended.word().into())),
                ("heard", Some((t.peak_db >= HEARD_DB).into())),
                ("peak", Some(f64::from(meter(t.peak_db)).into())),
                ("message", message.map(Into::into)),
                ("ref", t.id.map(Into::into)),
            ],
        ));
    }
}

/// **Worker.** Opens and starts the chosen mic for a test: the source, its ring, its path to
/// 16 kHz, and which mic it is.
fn open_test(
    shared: &Shared,
    capture: &dyn CaptureControl,
) -> Result<
    (
        Box<dyn AudioSource>,
        ink_audio::CaptureConsumer,
        MicPath,
        MicInfo,
    ),
    String,
> {
    let picked = devices::pick_mic(capture, &devices::input_choice(shared.store.as_ref()))?;
    let mut source = capture
        .open_mic(Some(&picked.device.id))
        .map_err(|e| format!("the microphone {}: {e}", picked.device.name))?;
    let format = source.format();
    let (producer, ring) =
        capture_ring(format, ink_audio::DEFAULT_RING_DURATION).map_err(|e| e.to_string())?;
    let path = MicPath::new(format).map_err(|e| e.to_string())?;
    source
        .start(Box::new(producer))
        .map_err(|e| format!("the microphone {}: {e}", picked.device.name))?;
    Ok((source, ring, path, picked.mic_info(picked.device.transport)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_meter_is_linear_in_db_from_minus_60_to_full_scale() {
        assert_eq!(meter(f32::NEG_INFINITY), 0.0);
        assert_eq!(meter(-80.0), 0.0);
        assert_eq!(meter(-60.0), 0.0);
        assert!((meter(-30.0) - 0.5).abs() < 1e-6);
        assert_eq!(meter(0.0), 1.0);
        assert_eq!(meter(6.0), 1.0);
        assert_eq!(level_db(Bands::default()), f32::NEG_INFINITY);
        let full = Bands {
            low: 1.0,
            mid: 0.0,
            high: 0.0,
        };
        assert!(level_db(full).abs() < 1e-6);
        // Three bands at -20 dB each combine to about -15.2 dB.
        let each = Bands {
            low: 0.1,
            mid: 0.1,
            high: 0.1,
        };
        assert!((level_db(each) + 15.23).abs() < 0.01, "{}", level_db(each));
    }

    /// A storm of OS notifications queues one message while that one waits; once the thread has
    /// taken it (and cleared `pending`), the next notification queues one more.
    #[test]
    fn the_device_callback_keeps_at_most_one_change_queued() {
        let (tx, rx) = mpsc::channel();
        let pending = Arc::new(AtomicBool::new(false));
        let sink = change_sink(tx, pending.clone());
        for _ in 0..1_000 {
            sink(DeviceChange::Devices);
        }
        let queued: Vec<Msg> = rx.try_iter().collect();
        assert_eq!(queued.len(), 1);
        assert!(matches!(
            queued[0],
            Msg::Changed(Some(DeviceChange::Devices))
        ));
        sink(DeviceChange::DefaultInput);
        assert_eq!(rx.try_iter().count(), 0, "still pending");
        pending.store(false, Ordering::Release);
        sink(DeviceChange::DefaultOutput);
        sink(DeviceChange::DefaultOutput);
        assert_eq!(rx.try_iter().count(), 1);
    }

    #[test]
    fn the_commands_read_their_fields_and_refuse_others() {
        let read = |json: &str| {
            let v: Value = serde_json::from_str(json).unwrap();
            let name = v["cmd"].as_str().unwrap().to_owned();
            parse(&name, &v)
        };
        assert_eq!(
            read(r#"{"cmd":"audio.devices","id":"a"}"#),
            Some(Ok(SoundQuery::Devices))
        );
        assert_eq!(
            read(r#"{"cmd":"audio.test"}"#),
            Some(Ok(SoundQuery::Test(TEST_MAX)))
        );
        assert_eq!(
            read(r#"{"cmd":"audio.test","seconds":3}"#),
            Some(Ok(SoundQuery::Test(Duration::from_secs(3))))
        );
        for bad in [
            r#"{"cmd":"audio.test","seconds":0}"#,
            r#"{"cmd":"audio.test","seconds":16}"#,
            r#"{"cmd":"audio.test","seconds":1.5}"#,
            r#"{"cmd":"audio.devices","verbose":true}"#,
        ] {
            assert!(matches!(read(bad), Some(Err(_))), "{bad}");
        }
        assert_eq!(
            read(r#"{"cmd":"audio.test_stop"}"#),
            Some(Ok(SoundQuery::TestStop))
        );
        assert_eq!(read(r#"{"cmd":"audio.other"}"#), None);
    }
}
