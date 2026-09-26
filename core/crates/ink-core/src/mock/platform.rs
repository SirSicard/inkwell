use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::lock;
use crate::audio::{AudioBlock, AudioSink, AudioSource, Channel, SourceStats, StreamFormat};
use crate::clock::Clock;
use crate::error::PlatformError;
use crate::platform::{
    CaptureControl, DeviceId, DeviceInfo, FarEndTarget, FocusInfo, FocusReader, HotkeyBinding,
    HotkeyEvent, HotkeySource, InsertOutcome, MeetingDetector, MeetingSignal, Permission,
    PermissionProbe, PermissionState, Platform, TextInserter, Transport,
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
        Ok(SourceStats {
            frames,
            discontinuities: 0,
        })
    }
}

/// Every platform trait, driven from the test.
///
/// Defaults: one built-in default mic, built-in speakers as the output, 48 kHz mono mic and
/// 48 kHz stereo far end (so resampling and downmixing get exercised), every permission
/// `NotDetermined`, insertion answering `Pasted`. A permission set to `Denied` makes the calls that
/// need it fail with [`PlatformError::PermissionDenied`], as the real platforms do.
pub struct MockPlatform {
    clock: Arc<MockClock>,
    mic_format: StreamFormat,
    far_format: StreamFormat,
    inputs: Mutex<Vec<DeviceInfo>>,
    output: Mutex<Option<DeviceInfo>>,
    slots: Mutex<HashMap<Channel, Slot>>,
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
            output: Mutex::new(Some(device(
                "mock-speakers",
                "Built-in Speakers",
                Transport::BuiltIn,
            ))),
            slots: Mutex::default(),
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

    /// Replaces the device list and the default output.
    pub fn with_devices(self, inputs: Vec<DeviceInfo>, output: Option<DeviceInfo>) -> Self {
        *lock(&self.inputs) = inputs;
        *lock(&self.output) = output;
        self
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
    /// `host_time_ns`, as a capture callback would. Returns whether a started source took it.
    pub fn feed(&self, channel: Channel, samples: &[f32], host_time_ns: u64) -> bool {
        let Some(slot) = lock(&self.slots).get(&channel).cloned() else {
            return false;
        };
        // The slot stays locked across `push` on purpose: `stop` takes the same lock, so it cannot
        // return while a push is running, which is what `AudioSource::stop` promises.
        let mut slot = lock(&slot);
        let format = slot.format;
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

    fn open(&self, channel: Channel, format: StreamFormat) -> Box<dyn AudioSource> {
        let slot = Arc::new(Mutex::new(SinkSlot {
            format,
            sink: None,
            frames: 0,
        }));
        lock(&self.slots).insert(channel, slot.clone());
        Box::new(MockSource { channel, slot })
    }
}

impl CaptureControl for MockPlatform {
    fn input_devices(&self) -> Result<Vec<DeviceInfo>, PlatformError> {
        Ok(lock(&self.inputs).clone())
    }

    fn default_output(&self) -> Result<Option<DeviceInfo>, PlatformError> {
        Ok(lock(&self.output).clone())
    }

    fn open_mic(&self, device: Option<&DeviceId>) -> Result<Box<dyn AudioSource>, PlatformError> {
        self.require(Permission::Microphone)?;
        if let Some(id) = device
            && !lock(&self.inputs).iter().any(|d| &d.id == id)
        {
            return Err(PlatformError::Device(format!("no input device {}", id.0)));
        }
        Ok(self.open(Channel::Mic, self.mic_format))
    }

    fn open_far_end(&self, target: &FarEndTarget) -> Result<Box<dyn AudioSource>, PlatformError> {
        self.require(Permission::SystemAudio)?;
        lock(&self.far_targets).push(target.clone());
        Ok(self.open(Channel::Far, self.far_format))
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
