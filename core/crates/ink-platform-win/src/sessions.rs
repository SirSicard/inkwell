//! Audio sessions: which processes have a stream on which endpoint, and whether it is running.
//!
//! Every process that opens a WASAPI stream gets a session on that endpoint
//! (`IAudioSessionManager2`). A session on a **capture** endpoint in the `Active` state is a
//! process recording from that mic right now: meeting detection's signal. A session on a
//! **render** endpoint says where an app plays: the far end's device loopback follows it.
//!
//! **Worker**, inside the caller's COM scope.
#![cfg(windows)]

use ink_core::PlatformError;
use windows::Win32::Foundation::S_OK;
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2, IMMDevice,
    IMMDeviceEnumerator,
};
use windows::Win32::System::Com::CLSCTX_ALL;
use windows::core::Interface;

use crate::capture::devices::{self, Flow};
use crate::com::device_error;

/// One session, as far as this crate cares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Session {
    /// The process that owns it.
    pub(crate) pid: u32,
    /// Streaming now (`AudioSessionStateActive`), not merely opened.
    pub(crate) active: bool,
    /// The endpoint it lives on.
    pub(crate) endpoint: String,
}

/// One read of an endpoint's (or several endpoints') sessions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SessionScan {
    /// Every per-process session that could be read. The system-sounds session is never listed.
    pub(crate) sessions: Vec<Session>,
    /// Sessions attempted.
    pub(crate) reads: usize,
    /// Sessions whose pid or state could not be read.
    pub(crate) failed: usize,
}

impl SessionScan {
    /// Every read failed: the scan saw nothing it can trust. (No sessions at all is not blind; it
    /// is a quiet machine.)
    pub(crate) fn blind(&self) -> bool {
        self.reads > 0 && self.failed == self.reads
    }

    fn absorb(&mut self, other: SessionScan) {
        self.sessions.extend(other.sessions);
        self.reads += other.reads;
        self.failed += other.failed;
    }
}

/// The session manager of `device`.
pub(crate) fn manager(device: &IMMDevice) -> Result<IAudioSessionManager2, PlatformError> {
    // SAFETY: a live device; the manager is an in-process activation.
    unsafe { device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) }
        .map_err(|e| device_error("opening the audio session manager", &e))
}

/// The sessions on one endpoint.
pub(crate) fn read(
    manager: &IAudioSessionManager2,
    endpoint: &str,
) -> Result<SessionScan, PlatformError> {
    // SAFETY: a live manager.
    let list = unsafe { manager.GetSessionEnumerator() }
        .map_err(|e| device_error("listing audio sessions", &e))?;
    // SAFETY: a live enumerator.
    let count =
        unsafe { list.GetCount() }.map_err(|e| device_error("counting audio sessions", &e))?;
    let mut scan = SessionScan::default();
    for index in 0..count {
        scan.reads += 1;
        let one = || -> windows::core::Result<Option<Session>> {
            // SAFETY: `index < count` on a live enumerator.
            let control = unsafe { list.GetSession(index) }?;
            let control: IAudioSessionControl2 = control.cast()?;
            // SAFETY: a live session control. S_OK means the system-sounds session, which
            // belongs to no app.
            if unsafe { control.IsSystemSoundsSession() } == S_OK {
                return Ok(None);
            }
            // SAFETY: as above.
            let pid = unsafe { control.GetProcessId() }?;
            // SAFETY: as above.
            let state = unsafe { control.GetState() }?;
            Ok((pid != 0).then(|| Session {
                pid,
                active: state == AudioSessionStateActive,
                endpoint: endpoint.to_owned(),
            }))
        };
        match one() {
            Ok(Some(session)) => scan.sessions.push(session),
            Ok(None) => {}
            Err(_) => scan.failed += 1,
        }
    }
    Ok(scan)
}

/// The sessions on every active endpoint of `flow`. An endpoint that cannot be read counts as one
/// failed read; a failure to list the endpoints at all is an error.
pub(crate) fn read_all(
    devices: &IMMDeviceEnumerator,
    flow: Flow,
) -> Result<SessionScan, PlatformError> {
    let mut scan = SessionScan::default();
    for endpoint in devices::endpoints(devices, flow)? {
        let id = endpoint.info.id.0;
        let read_one = || {
            let device = devices::endpoint_by_id(devices, &id)?;
            read(&manager(&device)?, &id)
        };
        match read_one() {
            Ok(one) => scan.absorb(one),
            Err(_) => {
                scan.reads += 1;
                scan.failed += 1;
            }
        }
    }
    Ok(scan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blind_means_every_read_failed() {
        let mut scan = SessionScan::default();
        assert!(!scan.blind(), "nothing to read is a quiet machine");
        scan.reads = 3;
        scan.failed = 2;
        assert!(!scan.blind());
        scan.failed = 3;
        assert!(scan.blind());
    }

    /// Talks to the Windows audio service. On the PC: `cargo test -p ink-platform-win -- --ignored`.
    #[test]
    #[ignore = "talks to the Windows audio service"]
    fn the_real_sessions_read_without_error() {
        let _com = crate::com::ComScope::enter().expect("COM");
        let devices = devices::enumerator().expect("MMDevice API");
        for flow in [Flow::Capture, Flow::Render] {
            let scan = read_all(&devices, flow).expect("a scan");
            assert!(!scan.blind(), "{flow:?}: {scan:?}");
        }
    }
}
