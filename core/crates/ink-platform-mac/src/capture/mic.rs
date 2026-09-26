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

use super::hal::{self, HalDevice, capture_format};
use super::io::{InputContext, IoCounters, IoStats, RunningIo, finish, input_proc};
use super::routing;
use crate::clock::MacClock;
use crate::permissions;

/// The microphone [`AudioSource`].
///
/// A Bluetooth headset mic delivers 16 kHz call audio and **digital zeros while the user is
/// silent**, so a high share of exact zeros is normal on it (see
/// [`CaptureHealth`](super::CaptureHealth)); on any other mic, zeros throughout mean no data.
pub struct MacMicSource {
    running: Option<RunningIo<InputContext>>,
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

    /// The counters so far. **Any thread** that holds the source.
    pub fn stats(&self) -> IoStats {
        self.counters.snapshot()
    }
}

/// The format buffer 0 of an IOProc on `device` carries.
fn input_format(device: &HalDevice) -> Result<StreamFormat, PlatformError> {
    let asbd = hal::first_stream_format(device.id, kAudioObjectPropertyScopeInput)?
        .ok_or_else(|| PlatformError::Device(format!("{} has no input stream", device.name)))?;
    capture_format(&asbd)
        .map_err(|problem| PlatformError::Device(format!("microphone {}: {problem}", device.name)))
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
    /// what makes macOS ask.
    fn start(&mut self, sink: Box<dyn AudioSink>) -> Result<(), PlatformError> {
        if self.running.is_some() {
            return Err(PlatformError::Failed(
                "the microphone is already started".into(),
            ));
        }
        if permissions::microphone() == PermissionState::Denied {
            return Err(PlatformError::PermissionDenied(Permission::Microphone));
        }
        // A device can change format between open and start (another app set its rate). Blocks
        // labelled with the old format would be resampled wrongly, so that is an error here.
        if input_format(&self.device)? != self.format {
            return Err(PlatformError::Device(format!(
                "microphone {} changed format since it was opened; open it again",
                self.device.name
            )));
        }
        let context = InputContext::new(
            sink,
            self.guard.clone(),
            self.format,
            self.clock,
            Arc::clone(&self.counters),
        );
        match RunningIo::start(self.device.id, Some(input_proc), context) {
            Ok(running) => {
                self.running = Some(running);
                Ok(())
            }
            Err((_context, error)) => Err(error.into()),
        }
    }

    fn stop(&mut self) -> Result<SourceStats, PlatformError> {
        let Some(running) = self.running.take() else {
            return Ok(SourceStats::default());
        };
        finish(running, &self.counters, "microphone")
    }
}
