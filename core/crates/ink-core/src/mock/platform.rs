use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::lock;
use crate::audio::{AudioBlock, AudioSink, AudioSource, Channel, SourceStats, StreamFormat};
use crate::clock::Clock;
use crate::error::PlatformError;
use crate::platform::{
    AutoInput, AutoReason, CaptureControl, DeviceChange, DeviceId, DeviceInfo, FarEndTarget,
    FocusInfo, FocusReader, HotkeyBinding, HotkeyEvent, HotkeySource, InsertOutcome,
    MeetingDetector, MeetingSignal, Permission, PermissionProbe, PermissionState, Platform,
    TextInserter, Transport,
};
use crate::threading::EventSink;

/// A clock the test sets. Lock-free, so it is the one mock that is realtime-safe.
///
/// Wall time is derived from host time against a fixed anchor, never accumulated separately, so
/// sub-millisecond steps add up instead of rounding away.
#[derive(Debug, Default)]
pub struct MockClock {
    now_ns: AtomicU64,
    anchor_ns: u64,
    anchor_unix_ms: i64,
}

impl MockClock {
    /// A clock at host time `now_ns` and wall time `unix_ms`.
    pub fn new(now_ns: u64, unix_ms: i64) -> Self {
        Self {
            now_ns: AtomicU64::new(now_ns),
            anchor_ns: now_ns,
            anchor_unix_ms: unix_ms,
        }
    }

    /// Moves host time forward; wall time follows.
    pub fn advance_ns(&self, ns: u64) {
        self.now_ns.fetch_add(ns, Ordering::AcqRel);
    }
}

impl Clock for MockClock {
    fn now_ns(&self) -> u64 {
        self.now_ns.load(Ordering::Acquire)
    }

    fn unix_ms(&self) -> i64 {
        let elapsed_ms = (self.now_ns() - self.anchor_ns) / 1_000_000;
        self.anchor_unix_ms
            .saturating_add(i64::try_from(elapsed_ms).unwrap_or(i64::MAX))
    }
}

struct SinkSlot {
    format: StreamFormat,
    sink: Option<Box<dyn AudioSink>>,
    frames: u64,
    /// The input it records (mic sources only).
    device: Option<DeviceId>,
    /// Its device went away ([`MockPlatform::unplug`]): nothing more is delivered, `ended` says
    /// so and `stop` says why.
    ended: bool,
}

type Slot = Arc<Mutex<SinkSlot>>;
type BoundHotkey = (HotkeyBinding, EventSink<HotkeyEvent>);

struct MockSource {
    channel: Channel,
    slot: Slot,
}

impl AudioSource for MockSource {
    fn channel(&self) -> Channel {
        self.channel
    }

    fn format(&self) -> StreamFormat {
        lock(&self.slot).format
    }

    fn start(&mut self, sink: Box<dyn AudioSink>) -> Result<(), PlatformError> {
        let mut slot = lock(&self.slot);
        if slot.sink.is_some() {
            return Err(PlatformError::Failed("source already started".into()));
        }
        slot.sink = Some(sink);
        slot.frames = 0;
        Ok(())
    }

    fn stop(&mut self) -> Result<SourceStats, PlatformError> {
        let mut slot = lock(&self.slot);
        let frames = if slot.sink.take().is_some() {
            slot.frames
        } else {
            0
        };
        if slot.ended {
            let id = slot.device.as_ref().map_or("the device", |d| d.0.as_str());
            return Err(PlatformError::Device(format!("{id} went away")));
        }
        Ok(SourceStats {
            frames,
            discontinuities: 0,
        })
    }

    fn ended(&self) -> bool {
        lock(&self.slot).ended
    }
}

/// Every platform trait, driven from the test.
///
/// Defaults: one built-in default mic, built-in speakers as the output, 48 kHz mono mic and
/// 48 kHz stereo far end (so resampling and downmixing get exercised), every permission
/// `NotDetermined`, insertion answering `Pasted`. A permission set to `Denied` makes the calls that
/// need it fail with [`PlatformError::PermissionDenied`], as the real platforms do.
///
/// Devices are scripted: [`plug`](Self::plug), [`unplug`](Self::unplug) (an open mic on that
/// device ends, as a real one does) and the defaults' changes tell the watcher
/// ([`CaptureControl::watch_devices`]) at once, on the caller's thread. Automatic is the default
/// input, else the first; the routing rules are the platforms' own and tested there.
pub struct MockPlatform {
    clock: Arc<MockClock>,
    mic_format: StreamFormat,
    far_format: StreamFormat,
    inputs: Mutex<Vec<DeviceInfo>>,
    /// Outputs, the default marked; `None`: the platform has no output picker (macOS).
    outputs: Mutex<Option<Vec<DeviceInfo>>>,
    slots: Mutex<HashMap<Channel, Slot>>,
    /// Every mic source opened so far (an unplug ends those on its device).
    mic_slots: Mutex<Vec<Slot>>,
    /// The device of every mic opened so far, in order.
    mic_opens: Mutex<Vec<DeviceId>>,
    watcher: Mutex<Option<EventSink<DeviceChange>>>,
    far_targets: Mutex<Vec<FarEndTarget>>,
    meetings: Mutex<Option<EventSink<MeetingSignal>>>,
    hotkey: Mutex<Option<BoundHotkey>>,
    hotkey_held: AtomicBool,
    inserted: Mutex<Vec<String>>,
    insert_outcome: Mutex<InsertOutcome>,
    focus: Mutex<FocusInfo>,
    selection: Mutex<Option<String>>,
    permissions: Mutex<HashMap<Permission, PermissionState>>,
    requests: Mutex<Vec<Permission>>,
}

fn device(id: &str, name: &str, transport: Transport) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(id.into()),
        name: name.into(),
        transport,
        is_default: true,
    }
}

impl Default for MockPlatform {
    fn default() -> Self {
        Self {
            clock: Arc::new(MockClock::default()),
            mic_format: StreamFormat {
                sample_rate: 48_000,
                channels: 1,
            },
            far_format: StreamFormat {
                sample_rate: 48_000,
                channels: 2,
            },
            inputs: Mutex::new(vec![device(
                "mock-mic",
                "Built-in Microphone",
                Transport::BuiltIn,
            )]),
            outputs: Mutex::new(Some(vec![device(
                "mock-speakers",
                "Built-in Speakers",
                Transport::BuiltIn,
            )])),
            slots: Mutex::default(),
            mic_slots: Mutex::default(),
            mic_opens: Mutex::default(),
            watcher: Mutex::default(),
            far_targets: Mutex::default(),
            meetings: Mutex::default(),
            hotkey: Mutex::default(),
            hotkey_held: AtomicBool::new(false),
            inserted: Mutex::default(),
            insert_outcome: Mutex::new(InsertOutcome::Pasted),
            focus: Mutex::default(),
            selection: Mutex::default(),
            permissions: Mutex::default(),
            requests: Mutex::default(),
        }
    }
}

impl MockPlatform {
    /// A platform with the defaults above.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the device list and the default output (the only output).
    pub fn with_devices(self, inputs: Vec<DeviceInfo>, output: Option<DeviceInfo>) -> Self {
        *lock(&self.inputs) = inputs;
        *lock(&self.outputs) = Some(output.into_iter().collect());
        self
    }

    /// Replaces the outputs (the default among them marked `is_default`); `None`: no output
    /// picker, as on macOS.
    pub fn with_outputs(self, outputs: Option<Vec<DeviceInfo>>) -> Self {
        *lock(&self.outputs) = outputs;
        self
    }

    /// Tells the watcher, if one is watching. Returns whether one was.
    pub fn notify_devices(&self, change: DeviceChange) -> bool {
        let sink = lock(&self.watcher).clone();
        sink.map(|s| s(change)).is_some()
    }

    /// Whether a watcher is watching.
    pub fn watching_devices(&self) -> bool {
        lock(&self.watcher).is_some()
    }

    /// Connects an input (last in the list, or first and the default when it `is_default`), and
    /// tells the watcher.
    pub fn plug(&self, input: DeviceInfo) {
        {
            let mut inputs = lock(&self.inputs);
            inputs.retain(|d| d.id != input.id);
            if input.is_default {
                for d in inputs.iter_mut() {
                    d.is_default = false;
                }
                inputs.insert(0, input);
            } else {
                inputs.push(input);
            }
        }
        self.notify_devices(DeviceChange::Devices);
    }

    /// Disconnects the input or output `id`: it leaves the lists, every open mic on it ends (no
    /// more audio; `ended` is true and `stop` says why), and the watcher is told. Returns whether
    /// it was connected.
    pub fn unplug(&self, id: &str) -> bool {
        let id = DeviceId(id.to_owned());
        let removed = {
            let mut inputs = lock(&self.inputs);
            let before = inputs.len();
            inputs.retain(|d| d.id != id);
            let mut removed = inputs.len() != before;
            if let Some(outputs) = lock(&self.outputs).as_mut() {
                let before = outputs.len();
                outputs.retain(|d| d.id != id);
                removed |= outputs.len() != before;
            }
            removed
        };
        for slot in lock(&self.mic_slots).iter() {
            let mut slot = lock(slot);
            if slot.device.as_ref() == Some(&id) {
                slot.ended = true;
            }
        }
        if removed {
            self.notify_devices(DeviceChange::Devices);
        }
        removed
    }

    /// Makes the input `id` the default, and tells the watcher. Returns whether it is connected.
    pub fn set_default_input(&self, id: &str) -> bool {
        let found = {
            let mut inputs = lock(&self.inputs);
            let found = inputs.iter().any(|d| d.id.0 == id);
            if found {
                for d in inputs.iter_mut() {
                    d.is_default = d.id.0 == id;
                }
                inputs.sort_by_key(|d| !d.is_default);
            }
            found
        };
        if found {
            self.notify_devices(DeviceChange::DefaultInput);
        }
        found
    }

    /// Makes `output` the default output (added when it is new), and tells the watcher.
    pub fn set_default_output(&self, output: DeviceInfo) {
        {
            let mut outputs = lock(&self.outputs);
            let list = outputs.get_or_insert_with(Vec::new);
            list.retain(|d| d.id != output.id);
            for d in list.iter_mut() {
                d.is_default = false;
            }
            list.insert(
                0,
                DeviceInfo {
                    is_default: true,
                    ..output
                },
            );
        }
        self.notify_devices(DeviceChange::DefaultOutput);
    }

    /// The device of every mic opened so far, in order.
    pub fn mic_opens(&self) -> Vec<DeviceId> {
        lock(&self.mic_opens).clone()
    }

    /// Every service as trait objects over this one mock.
    pub fn platform(self: &Arc<Self>) -> Platform {
        Platform {
            capture: self.clone(),
            meetings: self.clone(),
            hotkeys: self.clone(),
            inserter: self.clone(),
            focus: self.clone(),
            permissions: self.clone(),
            clock: self.clock.clone(),
        }
    }

    /// The clock, to advance time.
    pub fn clock(&self) -> Arc<MockClock> {
        self.clock.clone()
    }

    /// Delivers `samples` to the most recently opened source on `channel`, stamped
    /// `host_time_ns`, as a capture callback would. Returns whether a started source took it (one
    /// whose device was unplugged takes nothing).
    pub fn feed(&self, channel: Channel, samples: &[f32], host_time_ns: u64) -> bool {
        let Some(slot) = lock(&self.slots).get(&channel).cloned() else {
            return false;
        };
        // The slot stays locked across `push` on purpose: `stop` takes the same lock, so it cannot
        // return while a push is running, which is what `AudioSource::stop` promises.
        let mut slot = lock(&slot);
        let format = slot.format;
        if slot.ended {
            return false;
        }
        let Some(sink) = slot.sink.as_mut() else {
            return false;
        };
        let block = AudioBlock {
            samples,
            format,
            host_time_ns,
        };
        sink.push(&block);
        slot.frames += block.frames() as u64;
        true
    }

    /// Every far-end target opened so far, in order.
    pub fn far_targets(&self) -> Vec<FarEndTarget> {
        lock(&self.far_targets).clone()
    }

    /// Sends a meeting signal. Returns whether a detector was listening.
    pub fn emit_meeting(&self, signal: MeetingSignal) -> bool {
        let sink = lock(&self.meetings).clone();
        sink.map(|s| s(signal)).is_some()
    }

    /// Detection stops on its own: sends [`MeetingSignal::Lost`] once and stops watching, as the
    /// real platforms do, so nothing arrives until `start` again. Returns whether a detector was
    /// listening.
    pub fn lose_meetings(&self, reason: &str) -> bool {
        let sink = lock(&self.meetings).take();
        sink.map(|s| {
            s(MeetingSignal::Lost {
                reason: reason.to_owned(),
            })
        })
        .is_some()
    }

    fn hotkey_event(&self, event: HotkeyEvent) -> bool {
        let sink = lock(&self.hotkey).as_ref().map(|(_, s)| s.clone());
        let delivered = sink.map(|s| s(event)).is_some();
        if delivered {
            let held = matches!(event, HotkeyEvent::Pressed { .. });
            self.hotkey_held.store(held, Ordering::Release);
        }
        delivered
    }

    /// Presses the hotkey now. Returns whether a binding was listening.
    pub fn press(&self) -> bool {
        self.hotkey_event(HotkeyEvent::Pressed {
            at_ns: self.clock.now_ns(),
        })
    }

    /// Releases the hotkey now. Returns whether a binding was listening.
    pub fn release(&self) -> bool {
        self.hotkey_event(HotkeyEvent::Released {
            at_ns: self.clock.now_ns(),
        })
    }

    /// Reports the event tap lost. Returns whether a binding was listening.
    pub fn cancel_hotkey(&self) -> bool {
        self.hotkey_event(HotkeyEvent::Cancelled)
    }

    /// The OS removes the hotkey: sends [`HotkeyEvent::Cancelled`] if a hold is in progress, then
    /// [`HotkeyEvent::Lost`], and unbinds, as the real
    /// platforms do, so nothing arrives until `start` again. Returns whether a binding was
    /// listening.
    pub fn lose_hotkey(&self) -> bool {
        let bound = lock(&self.hotkey).take();
        let was_held = self.hotkey_held.swap(false, Ordering::AcqRel);
        bound
            .map(|(_, sink)| {
                // The Mac tap ends a hold in progress before reporting the loss; so does the mock.
                if was_held {
                    sink(HotkeyEvent::Cancelled);
                }
                sink(HotkeyEvent::Lost)
            })
            .is_some()
    }

    /// The current binding, if the hotkey is started.
    pub fn hotkey_binding(&self) -> Option<HotkeyBinding> {
        lock(&self.hotkey).as_ref().map(|(b, _)| b.clone())
    }

    /// Everything inserted so far, in order.
    pub fn inserted(&self) -> Vec<String> {
        lock(&self.inserted).clone()
    }

    /// Sets what the next insertions report.
    pub fn set_insert_outcome(&self, outcome: InsertOutcome) {
        *lock(&self.insert_outcome) = outcome;
    }

    /// Sets the focus the reader reports.
    pub fn set_focus(&self, focus: FocusInfo) {
        *lock(&self.focus) = focus;
    }

    /// Sets the selected text the reader reports.
    pub fn set_selection(&self, selection: Option<&str>) {
        *lock(&self.selection) = selection.map(str::to_owned);
    }

    /// Sets a permission's state.
    pub fn set_permission(&self, permission: Permission, state: PermissionState) {
        lock(&self.permissions).insert(permission, state);
    }

    /// Every permission request so far, in order.
    pub fn requested(&self) -> Vec<Permission> {
        lock(&self.requests).clone()
    }

    fn require(&self, permission: Permission) -> Result<(), PlatformError> {
        if self.check(permission) == PermissionState::Denied {
            Err(PlatformError::PermissionDenied(permission))
        } else {
            Ok(())
        }
    }

    fn open(
        &self,
        channel: Channel,
        format: StreamFormat,
        device: Option<DeviceId>,
    ) -> Box<dyn AudioSource> {
        let slot = Arc::new(Mutex::new(SinkSlot {
            format,
            sink: None,
            frames: 0,
            device,
            ended: false,
        }));
        lock(&self.slots).insert(channel, slot.clone());
        if channel == Channel::Mic {
            lock(&self.mic_slots).push(slot.clone());
        }
        Box::new(MockSource { channel, slot })
    }
}

impl CaptureControl for MockPlatform {
    fn input_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError> {
        Ok(lock(&self.inputs).clone())
    }

    fn output_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError> {
        lock(&self.outputs)
            .clone()
            .ok_or(PlatformError::Unsupported("an output picker"))
    }

    fn default_output(&self) -> Result<Option<DeviceInfo>, PlatformError> {
        Ok(lock(&self.outputs)
            .as_ref()
            .and_then(|o| o.iter().find(|d| d.is_default).cloned()))
    }

    fn automatic_input(&self) -> Result<Option<AutoInput>, PlatformError> {
        let inputs = lock(&self.inputs);
        Ok(match inputs.iter().find(|d| d.is_default) {
            Some(d) => Some(AutoInput {
                device: d.clone(),
                reason: AutoReason::DefaultInput,
            }),
            None => inputs.first().map(|d| AutoInput {
                device: d.clone(),
                reason: AutoReason::FirstInput,
            }),
        })
    }

    fn open_mic(&self, device: Option<&DeviceId>) -> Result<Box<dyn AudioSource>, PlatformError> {
        self.require(Permission::Microphone)?;
        let id = match device {
            Some(id) => {
                if !lock(&self.inputs).iter().any(|d| &d.id == id) {
                    return Err(PlatformError::Device(format!("no input device {}", id.0)));
                }
                id.clone()
            }
            None => {
                self.automatic_input()?
                    .ok_or_else(|| PlatformError::Device("no input device".into()))?
                    .device
                    .id
            }
        };
        lock(&self.mic_opens).push(id.clone());
        Ok(self.open(Channel::Mic, self.mic_format, Some(id)))
    }

    fn open_far_end(&self, target: &FarEndTarget) -> Result<Box<dyn AudioSource>, PlatformError> {
        self.require(Permission::SystemAudio)?;
        lock(&self.far_targets).push(target.clone());
        Ok(self.open(Channel::Far, self.far_format, None))
    }

    fn watch_devices(&self, on_change: EventSink<DeviceChange>) -> Result<(), PlatformError> {
        *lock(&self.watcher) = Some(on_change);
        Ok(())
    }

    fn unwatch_devices(&self) {
        *lock(&self.watcher) = None;
    }
}

impl MeetingDetector for MockPlatform {
    fn start(&self, on_signal: EventSink<MeetingSignal>) -> Result<(), PlatformError> {
        *lock(&self.meetings) = Some(on_signal);
        Ok(())
    }

    fn stop(&self) {
        *lock(&self.meetings) = None;
    }
}

impl HotkeySource for MockPlatform {
    fn start(
        &self,
        binding: &HotkeyBinding,
        on_event: EventSink<HotkeyEvent>,
    ) -> Result<(), PlatformError> {
        // The macOS hotkey is an active event tap, which needs Accessibility.
        self.require(Permission::Accessibility)?;
        if binding.0.trim().is_empty() {
            return Err(PlatformError::Unsupported("an empty hotkey binding"));
        }
        *lock(&self.hotkey) = Some((binding.clone(), on_event));
        Ok(())
    }

    fn stop(&self) {
        *lock(&self.hotkey) = None;
    }
}

impl TextInserter for MockPlatform {
    fn insert(&self, text: &str) -> Result<InsertOutcome, PlatformError> {
        self.require(Permission::Accessibility)?;
        if lock(&self.focus).secure_input {
            return Ok(InsertOutcome::Blocked);
        }
        lock(&self.inserted).push(text.into());
        Ok(*lock(&self.insert_outcome))
    }
}

impl FocusReader for MockPlatform {
    fn focus(&self) -> Result<FocusInfo, PlatformError> {
        Ok(lock(&self.focus).clone())
    }

    fn selected_text(&self) -> Result<Option<String>, PlatformError> {
        Ok(lock(&self.selection).clone())
    }
}

impl PermissionProbe for MockPlatform {
    fn check(&self, permission: Permission) -> PermissionState {
        lock(&self.permissions)
            .get(&permission)
            .copied()
            .unwrap_or(PermissionState::NotDetermined)
    }

    fn request(&self, permission: Permission) -> Result<(), PlatformError> {
        lock(&self.requests).push(permission);
        Ok(())
    }
}
