//! The endpoint list, from the MMDevice API: what is plugged in, what each is called, how it
//! connects, and which is the default.
//!
//! **Worker** throughout; every call opens its own [`ComScope`](crate::com::ComScope) through its
//! caller. Enumeration needs no permission and opens no stream, so it runs anywhere, an SSH
//! session included.

use ink_core::{DeviceId, DeviceInfo, PlatformError};
use windows::Win32::Devices::FunctionDiscovery::{
    PKEY_Device_ContainerId, PKEY_Device_EnumeratorName, PKEY_Device_FriendlyName,
};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::{
    DEVICE_STATE_ACTIVE, EDataFlow, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    PKEY_AudioEndpoint_FormFactor, PKEY_AudioEngine_DeviceFormat, eCapture, eConsole, eRender,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PropVariantClear, PropVariantToStringAlloc, PropVariantToUInt32,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, STGM_READ};
use windows::Win32::System::Variant::VT_BLOB;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::core::HSTRING;

use super::routing::{Endpoint, transport};
use crate::com::{device_error, take_co_string};

/// Which way an endpoint carries audio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Flow {
    /// Capture endpoints: microphones, line in.
    Capture,
    /// Render endpoints: speakers, headphones.
    Render,
}

impl Flow {
    fn data_flow(self) -> EDataFlow {
        match self {
            Self::Capture => eCapture,
            Self::Render => eRender,
        }
    }

    fn what(self) -> &'static str {
        match self {
            Self::Capture => "input",
            Self::Render => "output",
        }
    }
}

/// The device enumerator. **Worker**, inside a COM scope.
pub(crate) fn enumerator() -> Result<IMMDeviceEnumerator, PlatformError> {
    // SAFETY: a registered in-process class; COM is initialised by the caller's scope.
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
        .map_err(|e| device_error("the audio device list is not available", &e))
}

/// An endpoint's id (`{0.0.1.00000000}.{guid}`).
pub(crate) fn endpoint_id(device: &IMMDevice) -> Result<String, PlatformError> {
    // SAFETY: GetId returns a CoTaskMem string that `take_co_string` frees.
    let raw = unsafe { device.GetId() }.map_err(|e| device_error("reading a device id", &e))?;
    // SAFETY: allocated by GetId, owned here.
    unsafe { take_co_string(raw) }
        .ok_or_else(|| PlatformError::Device("a device id that is not text".into()))
}

/// The default endpoint for `flow` (the console role), if one is set.
pub(crate) fn default_endpoint(
    devices: &IMMDeviceEnumerator,
    flow: Flow,
) -> Result<Option<IMMDevice>, PlatformError> {
    // SAFETY: plain enum arguments.
    match unsafe { devices.GetDefaultAudioEndpoint(flow.data_flow(), eConsole) } {
        Ok(device) => Ok(Some(device)),
        // E_NOTFOUND (HRESULT_FROM_WIN32(ERROR_NOT_FOUND)): no device of that kind at all.
        Err(e) if e.code().0 as u32 == 0x8007_0490 => Ok(None),
        Err(e) => Err(device_error(
            &format!("reading the default {}", flow.what()),
            &e,
        )),
    }
}

/// The endpoint with `id`, if it exists.
pub(crate) fn endpoint_by_id(
    devices: &IMMDeviceEnumerator,
    id: &str,
) -> Result<IMMDevice, PlatformError> {
    // SAFETY: a live wide string for the call.
    unsafe { devices.GetDevice(&HSTRING::from(id)) }
        .map_err(|e| device_error(&format!("no audio device with id {id}"), &e))
}

/// The active endpoints for `flow`, in the MMDevice API's order, each with what routing needs.
/// The default is marked; the caller sorts.
pub(crate) fn endpoints(
    devices: &IMMDeviceEnumerator,
    flow: Flow,
) -> Result<Vec<Endpoint>, PlatformError> {
    let default_id = default_endpoint(devices, flow)?
        .map(|d| endpoint_id(&d))
        .transpose()?;
    // SAFETY: plain arguments.
    let collection = unsafe { devices.EnumAudioEndpoints(flow.data_flow(), DEVICE_STATE_ACTIVE) }
        .map_err(|e| device_error(&format!("listing {} devices", flow.what()), &e))?;
    // SAFETY: a live collection.
    let count = unsafe { collection.GetCount() }
        .map_err(|e| device_error(&format!("counting {} devices", flow.what()), &e))?;
    let mut out = Vec::with_capacity(count as usize);
    for index in 0..count {
        // SAFETY: `index < count`.
        let Ok(device) = (unsafe { collection.Item(index) }) else {
            // Unplugged between the count and the read.
            continue;
        };
        let Ok(id) = endpoint_id(&device) else {
            continue;
        };
        let is_default = default_id.as_deref() == Some(id.as_str());
        out.push(describe(&device, id, is_default));
    }
    Ok(out)
}

/// What routing needs of one endpoint. A property that cannot be read leaves its field empty
/// rather than dropping the device: an unnamed mic is still a mic.
pub(crate) fn describe(device: &IMMDevice, id: String, is_default: bool) -> Endpoint {
    // SAFETY: a live device; read-only access.
    let store = unsafe { device.OpenPropertyStore(STGM_READ) }.ok();
    let string = |key: &PROPERTYKEY| store.as_ref().and_then(|s| read_string(s, key));
    let name = string(&PKEY_Device_FriendlyName).unwrap_or_else(|| id.clone());
    let enumerator = string(&PKEY_Device_EnumeratorName).unwrap_or_default();
    let container = string(&PKEY_Device_ContainerId);
    let form_factor = store
        .as_ref()
        .and_then(|s| read_u32(s, &PKEY_AudioEndpoint_FormFactor));
    let rate = store
        .as_ref()
        .and_then(|s| read_blob(s, &PKEY_AudioEngine_DeviceFormat))
        .and_then(|blob| format_rate(&blob));
    Endpoint {
        info: DeviceInfo {
            id: DeviceId(id),
            name,
            transport: transport(&enumerator, form_factor),
            is_default,
        },
        container,
        rate,
    }
}

/// Reads one property, hands it to `parse`, and clears it.
fn read<T>(
    store: &IPropertyStore,
    key: &PROPERTYKEY,
    parse: impl FnOnce(&PROPVARIANT) -> Option<T>,
) -> Option<T> {
    // SAFETY: a live store and key.
    let mut value = unsafe { store.GetValue(key) }.ok()?;
    let parsed = parse(&value);
    // SAFETY: `value` came from GetValue and is cleared once.
    let _ = unsafe { PropVariantClear(&mut value) };
    parsed
}

fn read_string(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    read(store, key, |value| {
        // SAFETY: a live PROPVARIANT; strings and GUIDs convert, anything else fails.
        let text = unsafe { PropVariantToStringAlloc(value) }.ok()?;
        // SAFETY: allocated by PropVariantToStringAlloc, owned here.
        unsafe { take_co_string(text) }.filter(|s| !s.is_empty())
    })
}

fn read_u32(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<u32> {
    // SAFETY: a live PROPVARIANT; a non-integer value fails.
    read(store, key, |value| {
        unsafe { PropVariantToUInt32(value) }.ok()
    })
}

fn read_blob(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<Vec<u8>> {
    read(store, key, |value| {
        // SAFETY: the union is read as a blob only when its tag says VT_BLOB; the blob's bytes
        // are live until the PROPVARIANT is cleared, after this copy.
        unsafe {
            let inner = &value.Anonymous.Anonymous;
            if inner.vt != VT_BLOB {
                return None;
            }
            let blob = inner.Anonymous.blob;
            if blob.pBlobData.is_null() {
                return None;
            }
            Some(std::slice::from_raw_parts(blob.pBlobData, blob.cbSize as usize).to_vec())
        }
    })
}

/// `nSamplesPerSec` from a `WAVEFORMATEX` (or `WAVEFORMATEXTENSIBLE`) in a byte blob, which may be
/// unaligned. `None` when the blob is too short or the rate is zero.
pub(crate) fn format_rate(blob: &[u8]) -> Option<u32> {
    // WAVEFORMATEX: wFormatTag u16, nChannels u16, nSamplesPerSec u32, ...
    let bytes: [u8; 4] = blob.get(4..8)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes)).filter(|&rate| rate > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::com::ComScope;

    #[test]
    fn a_format_blob_yields_its_rate() {
        // 32 kHz mono 16-bit PCM, as an LE Audio mic reports it.
        let mut blob = vec![1, 0, 1, 0];
        blob.extend_from_slice(&32_000u32.to_le_bytes());
        blob.extend_from_slice(&[0; 10]);
        assert_eq!(format_rate(&blob), Some(32_000));
        assert_eq!(format_rate(&blob[..7]), None, "too short");
        let mut zero = blob.clone();
        zero[4..8].copy_from_slice(&[0; 4]);
        assert_eq!(format_rate(&zero), None);
    }

    /// Talks to the Windows audio service, which a CI runner may not run. On the PC:
    /// `cargo test -p ink-platform-win -- --ignored`.
    #[test]
    #[ignore = "talks to the Windows audio service"]
    fn listing_endpoints_works_with_or_without_devices() {
        let _com = ComScope::enter().expect("COM");
        let devices = enumerator().expect("the MMDevice API");
        for flow in [Flow::Capture, Flow::Render] {
            let list = endpoints(&devices, flow).expect("a list, maybe empty");
            assert!(list.iter().filter(|e| e.info.is_default).count() <= 1);
        }
    }
}
