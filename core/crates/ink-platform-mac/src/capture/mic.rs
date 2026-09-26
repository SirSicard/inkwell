//! The microphone: an IOProc on the input device itself, never inside an aggregate.
//!
//! This is the mic half of the dual graph. A Bluetooth mic placed in an aggregate device delivers
//! digital zeros while the same mic works on its own, so the mic gets its own IOProc on its own
//! device and clock, and the far end its own tap-only aggregate; the two are aligned by the host
//! times on their blocks.
#![cfg(target_os = "macos")]

use std::sync::Arc;

use ink_audio::RealtimeGuard;
use ink_core::{
    AudioSink, AudioSource, Channel, Permission, PermissionState, PlatformError, SourceStats,
    StreamFormat, Transport,
};
use objc2_core_audio::kAudioObjectPropertyScopeInput;

use super::hal::{self, HalDevice, ObjectId, capture_format};
use super::io::{
    FormatListener, InputContext, IoCounters, IoStats, RunningIo, finish, start_input,
};
use super::routing;
use crate::clock::MacClock;
use crate::permissions;

/// The microphone [`AudioSource`].
///
/// A Bluetooth headset mic delivers 16 kHz call audio and **digital zeros while the user is
/// silent**, so a high share of exact zeros is normal on it (see
/// [`CaptureHealth`](ink_audio::CaptureHealth)); on any other mic, zeros throughout mean no data.
pub struct MacMicSource {
    // Field order is drop order: IO stops before the format listener goes.
    running: Option<RunningIo<InputContext>>,
    listener: Option<FormatListener>,
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
        result
    }
}
