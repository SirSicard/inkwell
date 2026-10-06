//! The microphone: an IOProc on the input device itself, never inside an aggregate.
//!
//! This is the mic half of the dual graph. A Bluetooth mic placed in an aggregate device delivers
//! digital zeros while the same mic works on its own, so the mic gets its own IOProc on its own
//! device and clock, and the far end its own tap-only aggregate; the two are aligned by the host
//! times on their blocks.
#![cfg(target_os = "macos")]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ink_audio::RealtimeGuard;
use ink_core::{
    AudioSink, AudioSource, Channel, Permission, PermissionState, PlatformError, SourceStats,
    StreamFormat, Transport,
};
use objc2_core_audio::{
    kAudioDevicePropertyDeviceIsAlive, kAudioObjectPropertyScopeGlobal,
    kAudioObjectPropertyScopeInput,
};

use super::hal::{self, HalDevice, ObjectId, capture_format};
use super::io::{
    ALIVE_PROPERTY, AliveContext, AliveReader, FormatListener, InputContext, IoCounters, IoHal,
    IoStats, Listeners, RunningIo, SYSTEM_IO, alive_listener, finish, start_input,
};
use super::routing;
use crate::clock::MacClock;
use crate::permissions;

/// The microphone [`AudioSource`].
///
/// A Bluetooth headset mic delivers 16 kHz call audio and **digital zeros while the user is
/// silent**, so a high share of exact zeros is normal on it (see
/// [`CaptureHealth`](ink_audio::CaptureHealth)); on any other mic, zeros throughout mean no data.
///
/// **A mic that goes** (unplugged, a headset switched off) is seen by a listener on the device's
/// `kAudioDevicePropertyDeviceIsAlive`: the source then reads as [`ended`](AudioSource::ended),
/// so a meeting opens the mic again elsewhere, and `stop` says the device went.
pub struct MacMicSource {
    // Field order is drop order: IO stops before the format and alive listeners go.
    running: Option<RunningIo<InputContext>>,
    listener: Option<FormatListener>,
    alive: Option<Listeners<AliveContext>>,
    /// Set by the alive listener (a HAL notification thread) when the device dies.
    gone: Arc<AtomicBool>,
    device: HalDevice,
    format: StreamFormat,
    clock: MacClock,
    guard: RealtimeGuard,
    counters: Arc<IoCounters>,
}

impl MacMicSource {
    /// Reads `device`'s input format. IO starts in `start`.
    pub(crate) fn open(
        device: HalDevice,
        clock: MacClock,
        guard: RealtimeGuard,
    ) -> Result<Self, PlatformError> {
        let format = input_format(&device)?;
        Ok(Self {
            running: None,
            listener: None,
            alive: None,
            gone: Arc::default(),
            device,
            format,
            clock,
            guard,
            counters: Arc::default(),
        })
    }

    /// The device's name, as macOS shows it.
    pub fn device_name(&self) -> &str {
        &self.device.name
    }

    /// How the device connects.
    pub fn transport(&self) -> Transport {
        routing::transport(self.device.transport)
    }

    /// The current (or last) session's counters. **Any thread** that holds the source.
    pub fn stats(&self) -> IoStats {
        self.counters.snapshot()
    }
}

/// The format buffer 0 of an IOProc on `device` carries.
fn input_format(device: &HalDevice) -> Result<StreamFormat, PlatformError> {
    read_input_format(device.id)
        .map_err(|problem| PlatformError::Device(format!("microphone {}: {problem}", device.name)))
}

/// Listens for `device` dying, marking `gone`, and looks once now (it may have gone since it was
/// opened).
pub(crate) fn watch_alive(
    hal: &'static dyn IoHal,
    device: ObjectId,
    read: AliveReader,
    gone: Arc<AtomicBool>,
) -> Result<Listeners<AliveContext>, hal::HalError> {
    // SAFETY: `alive_listener` reads its client data as an `AliveContext`, inside its gate.
    let listeners = unsafe {
        Listeners::register_on(
            hal,
            device,
            &ALIVE_PROPERTY,
            Some(alive_listener),
            AliveContext::new(read, gone),
            "listening for the microphone going away",
        )
    }?;
    listeners.context().check_now();
    Ok(listeners)
}

/// Whether `device` is alive, from the HAL.
fn read_alive(device: ObjectId) -> Result<bool, hal::HalError> {
    hal::get::<u32>(
        device,
        kAudioDevicePropertyDeviceIsAlive,
        kAudioObjectPropertyScopeGlobal,
        "reading whether the microphone is connected",
    )
    .map(|alive| alive != 0)
}

fn read_input_format(device: ObjectId) -> Result<StreamFormat, String> {
    let asbd = hal::first_stream_format(device, kAudioObjectPropertyScopeInput)
        .map_err(|e| e.to_string())?
        .ok_or("the device has no input stream")?;
    capture_format(&asbd).map_err(|problem| problem.to_string())
}

impl AudioSource for MacMicSource {
    fn channel(&self) -> Channel {
        Channel::Mic
    }

    fn format(&self) -> StreamFormat {
        self.format
    }

    /// Refuses with `PermissionDenied(Microphone)` when the permission is denied, rather than
    /// starting a capture that macOS would fill with zeros. When it was never asked, starting is
    /// what makes macOS ask. A device whose format changed since `open` is an error; one that
    /// changes during the session stops delivering, and `stop` says so.
    fn start(&mut self, sink: Box<dyn AudioSink>) -> Result<(), PlatformError> {
        if self.running.is_some() {
            return Err(PlatformError::Failed(
                "the microphone is already started".into(),
            ));
        }
        if permissions::microphone() == PermissionState::Denied {
            return Err(PlatformError::PermissionDenied(Permission::Microphone));
        }
        let id = self.device.id;
        self.counters = Arc::default(); // each session is judged on its own
        self.gone = Arc::default();
        let alive = watch_alive(
            &SYSTEM_IO,
            id,
            Arc::new(move || read_alive(id)),
            Arc::clone(&self.gone),
        )?;
        if self.gone.load(Ordering::Acquire) {
            return Err(PlatformError::Device(format!(
                "the microphone {} is not connected",
                self.device.name
            )));
        }
        let (running, listener) = start_input(
            id,
            self.format,
            sink,
            self.guard.clone(),
            self.clock,
            &self.counters,
            Arc::new(move || read_input_format(id)),
            "the microphone",
        )?;
        self.running = Some(running);
        self.listener = Some(listener);
        self.alive = Some(alive);
        Ok(())
    }

    fn stop(&mut self) -> Result<SourceStats, PlatformError> {
        let Some(running) = self.running.take() else {
            return Ok(SourceStats::default());
        };
        let result = finish(
            running,
            &self.counters,
            "microphone",
            self.format.sample_rate,
        );
        self.listener = None;
        self.alive = None;
        if self.gone.load(Ordering::Acquire) {
            // The device's going explains whatever teardown said about it.
            return Err(PlatformError::Device(format!(
                "the microphone {} went away mid-session after {} frames",
                self.device.name,
                self.counters.snapshot().frames
            )));
        }
        result
    }

    /// The device died (the alive listener), the format changed, or a panic stopped delivery:
    /// nothing more comes until the mic is opened again.
    fn ended(&self) -> bool {
        self.running.is_some()
            && (self.gone.load(Ordering::Acquire) || {
                let stats = self.counters.snapshot();
                stats.rate_changes > 0 || stats.panics > 0
            })
    }
}
