//! Process loopback: an `IAudioClient` that hears one process and its children, whatever device
//! they play to (Windows 10 2004 and later).
//!
//! It is activated through the virtual device `VAD\Process_Loopback`, asynchronously: Windows calls
//! a completion handler when the client is ready. The activation parameters and the wait follow
//! the wasapi crate's `new_application_loopback_client` (MIT, HEnquist/wasapi-rs), which S0.4's
//! probe used on this machine class.
//!
//! **Called on the capture thread**, inside its COM scope.
#![cfg(windows)]

use std::mem::ManuallyDrop;
use std::sync::mpsc;
use std::time::Duration;

use ink_core::PlatformError;
use windows::Win32::Media::Audio::{
    AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
    AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
    ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioClient, PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
    VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
};
use windows::Win32::System::Com::BLOB;
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Variant::VT_BLOB;
use windows::core::{HRESULT, IUnknown, Interface, Ref, implement};

use crate::com::device_error;

/// How long activation may take before it counts as failed.
const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(3);

/// The completion handler: it only says that activation finished. `#[implement]` objects are
/// agile, as `ActivateAudioInterfaceAsync` requires.
#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Completed(mpsc::SyncSender<()>);

impl IActivateAudioInterfaceCompletionHandler_Impl for Completed_Impl {
    fn ActivateCompleted(
        &self,
        _operation: Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        // A full channel or a gone receiver (activation timed out) needs nothing more.
        let _ = self.0.try_send(());
        Ok(())
    }
}

/// An uninitialised capture client on process loopback of `pid` and its process tree.
pub(crate) fn activate(pid: u32) -> Result<IAudioClient, PlatformError> {
    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    // A VT_BLOB pointing at `params`. It borrows `params` (never freed through the PROPVARIANT),
    // so it is never cleared: `ManuallyDrop` keeps it from being.
    let blob = ManuallyDrop::new(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                        pBlobData: std::ptr::from_mut(&mut params).cast(),
                    },
                },
            }),
        },
    });
    let (done, completed) = mpsc::sync_channel(1);
    let handler: IActivateAudioInterfaceCompletionHandler = Completed(done).into();
    // SAFETY: the device path is a static wide string; `blob` and the `params` it points at live
    // on this stack until after the wait below, which is as long as Windows reads them (it copies
    // them during the call); the handler is a live COM object.
    let operation = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(std::ptr::from_ref(&*blob)),
            &handler,
        )
    }
    .map_err(|e| device_error("activating process loopback", &e))?;
    if completed.recv_timeout(ACTIVATION_TIMEOUT).is_err() {
        return Err(PlatformError::Device(format!(
            "process loopback did not activate within {} s",
            ACTIVATION_TIMEOUT.as_secs()
        )));
    }
    let mut result = HRESULT(0);
    let mut client: Option<IUnknown> = None;
    // SAFETY: activation has completed; both out-pointers are live locals.
    unsafe { operation.GetActivateResult(&mut result, &mut client) }
        .map_err(|e| device_error("reading the process loopback activation", &e))?;
    result
        .ok()
        .map_err(|e| device_error(&format!("process loopback of pid {pid}"), &e))?;
    client
        .ok_or_else(|| PlatformError::Device("process loopback returned no client".into()))?
        .cast()
        .map_err(|e| device_error("process loopback's client", &e))
}
