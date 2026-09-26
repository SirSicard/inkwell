//! The Core Audio HAL as capture and detection use it: property reads on the system, device,
//! stream, process and tap objects, and the [`ProcessHal`] seam that lets the decisions built on
//! process objects be tested without an audio server.
//!
//! **Threads.** Worker only. Property reads are IPC to `coreaudiod` and may block briefly; none of
//! this runs on a realtime thread.
//!
//! **Permissions.** None of these reads needs one: listing devices and processes, and reading
//! whether a process runs input, work without Microphone or System Audio access.
#![cfg(target_os = "macos")]

use std::ffi::c_void;
use std::fmt;
use std::mem::{MaybeUninit, size_of};
use std::ptr::{self, NonNull};

use ink_core::{PlatformError, StreamFormat};
use objc2_core_audio::{
    AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID,
    AudioObjectPropertyAddress, AudioObjectPropertyScope, AudioObjectPropertySelector,
    kAudioDevicePropertyDeviceUID, kAudioDevicePropertyStreamConfiguration,
    kAudioDevicePropertyStreams, kAudioDevicePropertyTransportType, kAudioHardwareBadObjectError,
    kAudioHardwareBadPropertySizeError, kAudioHardwarePropertyDefaultInputDevice,
    kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyDevices,
    kAudioHardwarePropertyProcessObjectList, kAudioHardwarePropertyTranslatePIDToProcessObject,
    kAudioObjectPropertyElementMain, kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal,
    kAudioObjectPropertyScopeInput, kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject,
    kAudioObjectUnknown, kAudioProcessPropertyBundleID, kAudioProcessPropertyIsRunningInput,
    kAudioProcessPropertyPID, kAudioStreamPropertyVirtualFormat,
};
use objc2_core_audio_types::{
    AudioBuffer, AudioBufferList, AudioStreamBasicDescription, kAudioFormatFlagIsBigEndian,
    kAudioFormatFlagIsFloat, kAudioFormatFlagIsNonInterleaved, kAudioFormatLinearPCM,
};
use objc2_core_foundation::{CFRetained, CFString};

/// A HAL object: the system, a device, a stream, a process, a tap.
pub(crate) type ObjectId = AudioObjectID;

/// The system object, root of every query.
pub(crate) const SYSTEM: ObjectId = kAudioObjectSystemObject as ObjectId;

/// "No object", as the HAL answers for a missing default device or an unknown process.
pub(crate) const UNKNOWN: ObjectId = kAudioObjectUnknown;

/// A HAL call that failed: which question, and the `OSStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HalError {
    /// What was asked, for the message.
    pub what: &'static str,
    /// The `OSStatus`, usually a four-character code.
    pub status: i32,
}

impl HalError {
    /// The object is gone: a process that exited, a device that was unplugged, between listing it
    /// and reading it.
    pub(crate) fn is_gone(&self) -> bool {
        self.status == kAudioHardwareBadObjectError
    }
}

impl fmt::Display for HalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} failed: OSStatus {} ({})",
            self.what,
            self.status,
            four_cc(self.status)
        )
    }
}

impl From<HalError> for PlatformError {
    fn from(error: HalError) -> Self {
        PlatformError::Device(error.to_string())
    }
}

/// An `OSStatus` as its four-character code when it is one ('!obj', 'nope'), else the number:
/// Core Audio's errors are almost all four-character codes, and the integer alone is unreadable.
pub(crate) fn four_cc(status: i32) -> String {
    let bytes = status.to_be_bytes();
    if bytes.iter().all(|b| (0x20..0x7f).contains(b)) {
        format!(
            "'{}'",
            bytes.iter().map(|&b| char::from(b)).collect::<String>()
        )
    } else {
        status.to_string()
    }
}

/// Turns an `OSStatus` into a result.
pub(crate) fn check(status: i32, what: &'static str) -> Result<(), HalError> {
    if status == 0 {
        Ok(())
    } else {
        Err(HalError { what, status })
    }
}

/// Plain data a fixed-size property is copied into.
///
/// # Safety
///
/// Implement only for `Copy` types with no pointers or references, for which every bit pattern
/// (all zeros included) is a valid value: the HAL writes raw bytes into them.
pub(crate) unsafe trait Pod: Copy {}
// SAFETY: integers and floats accept every bit pattern and hold no pointers.
unsafe impl Pod for u32 {}
// SAFETY: as above.
unsafe impl Pod for i32 {}
// SAFETY: as above.
unsafe impl Pod for f64 {}
// SAFETY: a `repr(C)` struct of one f64 and eight u32s: no pointers, every bit pattern valid.
unsafe impl Pod for AudioStreamBasicDescription {}

fn address(
    selector: AudioObjectPropertySelector,
    scope: AudioObjectPropertyScope,
) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain,
    }
}

/// Reads a fixed-size property, optionally qualified (the pid for `'id2p'`).
fn get_qualified<T: Pod, Q: Pod>(
    object: ObjectId,
    selector: AudioObjectPropertySelector,
    scope: AudioObjectPropertyScope,
    qualifier: Option<&Q>,
    what: &'static str,
) -> Result<T, HalError> {
    let address = address(selector, scope);
    let mut value = MaybeUninit::<T>::zeroed();
    let mut size = size_of::<T>() as u32;
    let (qualifier_size, qualifier_ptr) = match qualifier {
        Some(q) => (size_of::<Q>() as u32, ptr::from_ref(q).cast::<c_void>()),
        None => (0, ptr::null()),
    };
    // SAFETY: `address` and `size` are live locals; `value` has room for `size` bytes, which is
    // what the HAL may write; the qualifier, when present, is a live `Q` of the size passed.
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&address),
            qualifier_size,
            qualifier_ptr,
            NonNull::from(&mut size),
            NonNull::from(&mut value).cast(),
        )
    };
    check(status, what)?;
    if size as usize != size_of::<T>() {
        return Err(HalError {
            what,
            status: kAudioHardwareBadPropertySizeError,
        });
    }
    // SAFETY: zero-initialised, then fully written by the HAL (the size matched); `T: Pod`
    // accepts every bit pattern.
    Ok(unsafe { value.assume_init() })
}

/// Reads a fixed-size property.
pub(crate) fn get<T: Pod>(
    object: ObjectId,
    selector: AudioObjectPropertySelector,
    scope: AudioObjectPropertyScope,
    what: &'static str,
) -> Result<T, HalError> {
    get_qualified::<T, u32>(object, selector, scope, None, what)
}

/// Reads a `CFString` property (a name, a UID, a bundle id). An empty string when the HAL has
/// none.
pub(crate) fn get_string(
    object: ObjectId,
    selector: AudioObjectPropertySelector,
    scope: AudioObjectPropertyScope,
    what: &'static str,
) -> Result<String, HalError> {
    let address = address(selector, scope);
    let mut raw: *const CFString = ptr::null();
    let mut size = size_of::<*const CFString>() as u32;
    // SAFETY: `raw` has room for one pointer, the size passed.
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&address),
            0,
            ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut raw).cast(),
        )
    };
    check(status, what)?;
    let Some(string) = NonNull::new(raw.cast_mut()) else {
        return Ok(String::new());
    };
    // SAFETY: for these string properties the HAL returns a +1 `CFStringRef` that the caller
    // releases (the "CFString" properties in `AudioHardware.h`); `from_raw` takes that reference.
    let string = unsafe { CFRetained::from_raw(string) };
    Ok(string.to_string())
}

/// Reads a variable-length property into 8-byte-aligned storage. Retries when the property grew
/// between the size query and the read (a device or process appearing).
fn get_bytes(
    object: ObjectId,
    selector: AudioObjectPropertySelector,
    scope: AudioObjectPropertyScope,
    what: &'static str,
) -> Result<(Vec<u64>, usize), HalError> {
    let address = address(selector, scope);
    let mut last = HalError {
        what,
        status: kAudioHardwareBadPropertySizeError,
    };
    for _ in 0..3 {
        let mut size = 0u32;
        // SAFETY: `address` and `size` are live locals; no qualifier.
        let status = unsafe {
            AudioObjectGetPropertyDataSize(
                object,
                NonNull::from(&address),
                0,
                ptr::null(),
                NonNull::from(&mut size),
            )
        };
        check(status, what)?;
        if size == 0 {
            return Ok((Vec::new(), 0));
        }
        let mut storage = vec![0u64; (size as usize).div_ceil(size_of::<u64>())];
        // SAFETY: `storage` holds at least `size` bytes, aligned to 8 (enough for every HAL
        // struct, `AudioBufferList` included).
        let status = unsafe {
            AudioObjectGetPropertyData(
                object,
                NonNull::from(&address),
                0,
                ptr::null(),
                NonNull::from(&mut size),
                NonNull::new_unchecked(storage.as_mut_ptr().cast::<c_void>()),
            )
        };
        match check(status, what) {
            Ok(()) => return Ok((storage, size as usize)),
            Err(e) if e.status == kAudioHardwareBadPropertySizeError => last = e,
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// Reads a list of object ids.
pub(crate) fn get_ids(
    object: ObjectId,
    selector: AudioObjectPropertySelector,
    scope: AudioObjectPropertyScope,
    what: &'static str,
) -> Result<Vec<ObjectId>, HalError> {
    let (storage, size) = get_bytes(object, selector, scope, what)?;
    let count = size / size_of::<ObjectId>();
    // SAFETY: `storage` holds at least `size` initialised bytes, aligned for u64 and so for u32,
    // and every bit pattern is a valid u32.
    let ids = unsafe { std::slice::from_raw_parts(storage.as_ptr().cast::<ObjectId>(), count) };
    Ok(ids.to_vec())
}

/// The channel count of each buffer in a device's stream configuration, in buffer order.
pub(crate) fn channels_per_buffer(
    device: ObjectId,
    scope: AudioObjectPropertyScope,
) -> Result<Vec<u32>, HalError> {
    const WHAT: &str = "reading a device's stream configuration";
    let (storage, size) = get_bytes(device, kAudioDevicePropertyStreamConfiguration, scope, WHAT)?;
    if size < size_of::<u32>() {
        return Ok(Vec::new());
    }
    let list = storage.as_ptr().cast::<AudioBufferList>();
    // SAFETY: `storage` is 8-aligned and holds `size` bytes written by the HAL, which is at least
    // the `mNumberBuffers` field.
    let count = unsafe { (*list).mNumberBuffers } as usize;
    let header = std::mem::offset_of!(AudioBufferList, mBuffers);
    if header + count * size_of::<AudioBuffer>() > size {
        return Err(HalError {
            what: WHAT,
            status: kAudioHardwareBadPropertySizeError,
        });
    }
    // SAFETY: bounds checked above: `count` buffers follow the header inside `storage`. The list
    // is variable-length, so the buffers are reached through a raw pointer, not the `[_; 1]`.
    let buffers = unsafe { ptr::addr_of!((*list).mBuffers).cast::<AudioBuffer>() };
    Ok((0..count)
        // SAFETY: `i < count`, in bounds as checked.
        .map(|i| unsafe { (*buffers.add(i)).mNumberChannels })
        .collect())
}

/// The virtual format of a device's first stream in `scope`: what an IOProc on it receives in
/// buffer 0. `None` when the device has no stream in that direction.
pub(crate) fn first_stream_format(
    device: ObjectId,
    scope: AudioObjectPropertyScope,
) -> Result<Option<AudioStreamBasicDescription>, HalError> {
    let streams = get_ids(
        device,
        kAudioDevicePropertyStreams,
        scope,
        "listing a device's streams",
    )?;
    let Some(&first) = streams.first() else {
        return Ok(None);
    };
    get(
        first,
        kAudioStreamPropertyVirtualFormat,
        kAudioObjectPropertyScopeGlobal,
        "reading a stream's format",
    )
    .map(Some)
}

/// Why a stream's format cannot be captured as it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum FormatProblem {
    /// Not linear PCM.
    NotPcm,
    /// Not native-endian 32-bit float. Reading integer samples as floats would produce noise
    /// that looks like audio, so it is refused, never guessed.
    NotFloat32,
    /// No sample rate or no channels.
    Empty,
}

impl fmt::Display for FormatProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotPcm => "the stream is not linear PCM",
            Self::NotFloat32 => "the stream is not native-endian 32-bit float",
            Self::Empty => "the stream reports no sample rate or no channels",
        })
    }
}

/// The [`StreamFormat`] that buffer 0 of an IOProc carries, for a stream in `asbd`'s format.
///
/// A non-interleaved stream puts one channel in each buffer, so buffer 0 is its first channel,
/// mono. The mic's downmix keeps the primary channel anyway, and the far-end tap is mono.
pub(crate) fn capture_format(
    asbd: &AudioStreamBasicDescription,
) -> Result<StreamFormat, FormatProblem> {
    if asbd.mFormatID != kAudioFormatLinearPCM {
        return Err(FormatProblem::NotPcm);
    }
    let flags = asbd.mFormatFlags;
    if flags & kAudioFormatFlagIsFloat == 0
        || flags & kAudioFormatFlagIsBigEndian != 0
        || asbd.mBitsPerChannel != 32
    {
        return Err(FormatProblem::NotFloat32);
    }
    let channels = if flags & kAudioFormatFlagIsNonInterleaved != 0 {
        1
    } else {
        asbd.mChannelsPerFrame
    };
    let rate = asbd.mSampleRate.round();
    if !(1.0..=f64::from(u32::MAX)).contains(&rate) || channels == 0 {
        return Err(FormatProblem::Empty);
    }
    Ok(StreamFormat {
        sample_rate: rate as u32,
        channels: u16::try_from(channels).map_err(|_| FormatProblem::Empty)?,
    })
}

/// A device as capture sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HalDevice {
    /// The HAL object, valid while the device is attached.
    pub id: ObjectId,
    /// The persistent UID (`DeviceId` in the core).
    pub uid: String,
    /// The name macOS shows.
    pub name: String,
    /// `kAudioDevicePropertyTransportType`, raw; `0` (unknown) when it would not say.
    pub transport: u32,
    /// Input channels over all input streams.
    pub input_channels: u32,
    /// Output channels over all output streams.
    pub output_channels: u32,
}

/// Every device, as the HAL lists them. A device that vanishes while it is being read is left
/// out; any other failure fails the listing, so an unreadable device is never mistaken for an
/// absent one.
pub(crate) fn devices() -> Result<Vec<HalDevice>, HalError> {
    let ids = get_ids(
        SYSTEM,
        kAudioHardwarePropertyDevices,
        kAudioObjectPropertyScopeGlobal,
        "listing audio devices",
    )?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        match device(id) {
            Ok(device) => out.push(device),
            Err(e) if e.is_gone() => {}
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

/// One device's identity, transport and channel counts.
pub(crate) fn device(id: ObjectId) -> Result<HalDevice, HalError> {
    let sum = |scope| channels_per_buffer(id, scope).map(|c| c.iter().sum::<u32>());
    Ok(HalDevice {
        id,
        uid: get_string(
            id,
            kAudioDevicePropertyDeviceUID,
            kAudioObjectPropertyScopeGlobal,
            "reading a device's UID",
        )?,
        name: get_string(
            id,
            kAudioObjectPropertyName,
            kAudioObjectPropertyScopeGlobal,
            "reading a device's name",
        )?,
        // Some virtual devices do not implement the transport type. Unknown is an answer
        // (`Transport::Other`), not a failure; the device stays listed.
        transport: match get::<u32>(
            id,
            kAudioDevicePropertyTransportType,
            kAudioObjectPropertyScopeGlobal,
            "reading a device's transport",
        ) {
            Ok(t) => t,
            Err(e) if e.is_gone() => return Err(e),
            Err(_) => 0,
        },
        input_channels: sum(kAudioObjectPropertyScopeInput)?,
        output_channels: sum(kAudioObjectPropertyScopeOutput)?,
    })
}

/// Which default device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Direction {
    /// The default input (microphone).
    Input,
    /// The default output.
    Output,
}

/// The default device for `direction`, or `None` when there is none (a Mac mini with nothing
/// plugged in, a CI runner).
pub(crate) fn default_device(direction: Direction) -> Result<Option<ObjectId>, HalError> {
    let (selector, what) = match direction {
        Direction::Input => (
            kAudioHardwarePropertyDefaultInputDevice,
            "reading the default input device",
        ),
        Direction::Output => (
            kAudioHardwarePropertyDefaultOutputDevice,
            "reading the default output device",
        ),
    };
    let id: ObjectId = get(SYSTEM, selector, kAudioObjectPropertyScopeGlobal, what)?;
    Ok((id != UNKNOWN).then_some(id))
}

/// The HAL's process objects, behind a seam: meeting detection and per-app taps are decisions over
/// these reads, and the seam lets tests drive them with made-up processes.
///
/// A HAL **process object id is not a pid.** Both are `u32`-sized, so passing a pid where the HAL
/// wants a process object compiles, runs, and quietly names some other object.
pub(crate) trait ProcessHal: Send + Sync {
    /// Every process object the HAL knows (`kAudioHardwarePropertyProcessObjectList`).
    fn process_objects(&self) -> Result<Vec<ObjectId>, HalError>;
    /// A process object's pid (`kAudioProcessPropertyPID`).
    fn pid(&self, process: ObjectId) -> Result<i32, HalError>;
    /// A process object's bundle id (`kAudioProcessPropertyBundleID`); empty for a tool without
    /// a bundle.
    fn bundle_id(&self, process: ObjectId) -> Result<String, HalError>;
    /// Whether the process has an input stream running (`kAudioProcessPropertyIsRunningInput`).
    fn is_running_input(&self, process: ObjectId) -> Result<bool, HalError>;
    /// The process object for a pid (`kAudioHardwarePropertyTranslatePIDToProcessObject`).
    /// `None` when the process has none (it has never talked to the audio server).
    fn process_object_for_pid(&self, pid: i32) -> Result<Option<ObjectId>, HalError>;
}

/// [`ProcessHal`] on the real audio server.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SystemHal;

impl ProcessHal for SystemHal {
    fn process_objects(&self) -> Result<Vec<ObjectId>, HalError> {
        get_ids(
            SYSTEM,
            kAudioHardwarePropertyProcessObjectList,
            kAudioObjectPropertyScopeGlobal,
            "listing audio processes",
        )
    }

    fn pid(&self, process: ObjectId) -> Result<i32, HalError> {
        get(
            process,
            kAudioProcessPropertyPID,
            kAudioObjectPropertyScopeGlobal,
            "reading a process's pid",
        )
    }

    fn bundle_id(&self, process: ObjectId) -> Result<String, HalError> {
        get_string(
            process,
            kAudioProcessPropertyBundleID,
            kAudioObjectPropertyScopeGlobal,
            "reading a process's bundle id",
        )
    }

    fn is_running_input(&self, process: ObjectId) -> Result<bool, HalError> {
        get::<u32>(
            process,
            kAudioProcessPropertyIsRunningInput,
            kAudioObjectPropertyScopeGlobal,
            "reading whether a process runs input",
        )
        .map(|running| running != 0)
    }

    fn process_object_for_pid(&self, pid: i32) -> Result<Option<ObjectId>, HalError> {
        let object: ObjectId = get_qualified(
            SYSTEM,
            kAudioHardwarePropertyTranslatePIDToProcessObject,
            kAudioObjectPropertyScopeGlobal,
            Some(&pid),
            "translating a pid to a process object",
        )?;
        Ok((object != UNKNOWN).then_some(object))
    }
}

/// Whether `bundle_id` belongs to the app `app_id`: the app itself, or a helper registered under
/// its id (`com.example.Browser.helper` belongs to `com.example.Browser`). Browsers and some
/// meeting apps play audio from such helpers.
pub(crate) fn belongs_to_app(bundle_id: &str, app_id: &str) -> bool {
    !app_id.is_empty()
        && (bundle_id == app_id
            || bundle_id
                .strip_prefix(app_id)
                .is_some_and(|rest| rest.starts_with('.')))
}

#[cfg(test)]
pub(crate) mod fake {
    //! A made-up audio server for tests: process objects with ids deliberately unlike their pids.

    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use super::*;

    /// One process as the fake HAL reports it. `None` fields fail their read.
    #[derive(Clone, Debug)]
    pub(crate) struct FakeProcess {
        pub pid: i32,
        pub bundle_id: Option<String>,
        pub running_input: Option<bool>,
    }

    /// A scriptable [`ProcessHal`].
    #[derive(Default)]
    pub(crate) struct FakeHal {
        pub processes: Mutex<BTreeMap<ObjectId, FakeProcess>>,
        /// When set, listing the processes fails with this status.
        pub list_fails: Mutex<Option<i32>>,
    }

    const BROKEN: i32 = i32::from_be_bytes(*b"what");

    impl FakeHal {
        pub(crate) fn with(processes: &[(ObjectId, i32, &str, bool)]) -> Self {
            let hal = Self::default();
            for &(object, pid, bundle, running) in processes {
                hal.set(object, pid, bundle, running);
            }
            hal
        }

        pub(crate) fn set(&self, object: ObjectId, pid: i32, bundle: &str, running: bool) {
            self.processes.lock().unwrap().insert(
                object,
                FakeProcess {
                    pid,
                    bundle_id: Some(bundle.to_owned()),
                    running_input: Some(running),
                },
            );
        }

        pub(crate) fn remove(&self, object: ObjectId) {
            self.processes.lock().unwrap().remove(&object);
        }

        fn process(&self, object: ObjectId) -> Result<FakeProcess, HalError> {
            self.processes
                .lock()
                .unwrap()
                .get(&object)
                .cloned()
                .ok_or(HalError {
                    what: "fake",
                    status: kAudioHardwareBadObjectError,
                })
        }
    }

    impl ProcessHal for FakeHal {
        fn process_objects(&self) -> Result<Vec<ObjectId>, HalError> {
            if let Some(status) = *self.list_fails.lock().unwrap() {
                return Err(HalError {
                    what: "fake list",
                    status,
                });
            }
            Ok(self.processes.lock().unwrap().keys().copied().collect())
        }

        fn pid(&self, process: ObjectId) -> Result<i32, HalError> {
            Ok(self.process(process)?.pid)
        }

        fn bundle_id(&self, process: ObjectId) -> Result<String, HalError> {
            self.process(process)?.bundle_id.ok_or(HalError {
                what: "fake bundle",
                status: BROKEN,
            })
        }

        fn is_running_input(&self, process: ObjectId) -> Result<bool, HalError> {
            self.process(process)?.running_input.ok_or(HalError {
                what: "fake piri",
                status: BROKEN,
            })
        }

        fn process_object_for_pid(&self, pid: i32) -> Result<Option<ObjectId>, HalError> {
            Ok(self
                .processes
                .lock()
                .unwrap()
                .iter()
                .find(|(_, p)| p.pid == pid)
                .map(|(&object, _)| object))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asbd(flags: u32, bits: u32, channels: u32, rate: f64) -> AudioStreamBasicDescription {
        AudioStreamBasicDescription {
            mSampleRate: rate,
            mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: flags,
            mBytesPerPacket: bits / 8 * channels,
            mFramesPerPacket: 1,
            mBytesPerFrame: bits / 8 * channels,
            mChannelsPerFrame: channels,
            mBitsPerChannel: bits,
            mReserved: 0,
        }
    }

    #[test]
    fn a_float32_stream_is_captured_as_it_is() {
        let format = capture_format(&asbd(kAudioFormatFlagIsFloat, 32, 2, 48_000.0));
        assert_eq!(
            format,
            Ok(StreamFormat {
                sample_rate: 48_000,
                channels: 2
            })
        );
    }

    #[test]
    fn integer_or_big_endian_samples_are_refused_rather_than_misread() {
        assert_eq!(
            capture_format(&asbd(0, 16, 1, 16_000.0)),
            Err(FormatProblem::NotFloat32)
        );
        assert_eq!(
            capture_format(&asbd(0, 32, 1, 16_000.0)),
            Err(FormatProblem::NotFloat32),
            "32-bit integers are not floats"
        );
        assert_eq!(
            capture_format(&asbd(
                kAudioFormatFlagIsFloat | kAudioFormatFlagIsBigEndian,
                32,
                1,
                16_000.0
            )),
            Err(FormatProblem::NotFloat32)
        );
        let mut not_pcm = asbd(kAudioFormatFlagIsFloat, 32, 1, 16_000.0);
        not_pcm.mFormatID = u32::from_be_bytes(*b"aac ");
        assert_eq!(capture_format(&not_pcm), Err(FormatProblem::NotPcm));
    }

    #[test]
    fn a_non_interleaved_stream_delivers_its_first_channel_in_buffer_zero() {
        let format = capture_format(&asbd(
            kAudioFormatFlagIsFloat | kAudioFormatFlagIsNonInterleaved,
            32,
            4,
            44_100.0,
        ));
        assert_eq!(format.map(|f| f.channels), Ok(1));
    }

    #[test]
    fn a_stream_without_rate_or_channels_is_refused() {
        assert_eq!(
            capture_format(&asbd(kAudioFormatFlagIsFloat, 32, 0, 48_000.0)),
            Err(FormatProblem::Empty)
        );
        assert_eq!(
            capture_format(&asbd(kAudioFormatFlagIsFloat, 32, 1, 0.0)),
            Err(FormatProblem::Empty)
        );
    }

    #[test]
    fn statuses_read_as_four_character_codes() {
        assert_eq!(four_cc(kAudioHardwareBadObjectError), "'!obj'");
        assert_eq!(four_cc(-50), "-50");
        let error = HalError {
            what: "reading x",
            status: kAudioHardwareBadObjectError,
        };
        assert!(error.is_gone());
        assert_eq!(
            PlatformError::from(error),
            PlatformError::Device("reading x failed: OSStatus 560947818 ('!obj')".into())
        );
    }

    #[test]
    fn helpers_belong_to_their_app_and_lookalikes_do_not() {
        assert!(belongs_to_app("us.zoom.xos", "us.zoom.xos"));
        assert!(belongs_to_app(
            "com.example.Browser.helper",
            "com.example.Browser"
        ));
        assert!(!belongs_to_app(
            "com.example.BrowserBeta",
            "com.example.Browser"
        ));
        assert!(!belongs_to_app(
            "com.example.Browser",
            "com.example.Browser.helper"
        ));
        assert!(!belongs_to_app("anything", ""));
    }

    // The next tests talk to the real audio server. They need no permission, but a CI runner
    // may have no audio server worth asking, so they run locally: `cargo test -- --ignored`.

    #[test]
    #[ignore = "talks to the local audio server"]
    fn the_real_server_lists_devices_and_processes() {
        let devices = devices().expect("device list");
        for device in &devices {
            assert!(!device.uid.is_empty(), "{device:?}");
        }
        let processes = SystemHal.process_objects().expect("process list");
        assert!(!processes.is_empty(), "at least coreaudiod's clients");
        for &process in &processes {
            // A process may exit between the list and the read; anything else is a bug.
            match SystemHal.pid(process) {
                Ok(pid) => assert!(pid > 0, "pid {pid}"),
                Err(e) => assert!(e.is_gone(), "{e}"),
            }
        }
    }

    #[test]
    #[ignore = "talks to the local audio server"]
    fn our_own_pid_translates_to_a_process_object_that_is_not_the_pid() {
        // Talking to the server at all makes this process one of its clients.
        let _ = devices();
        let pid = std::process::id() as i32;
        let object = SystemHal
            .process_object_for_pid(pid)
            .expect("translation")
            .expect("a process object for a client of the audio server");
        assert_eq!(SystemHal.pid(object), Ok(pid));
        assert_ne!(object as i64, i64::from(pid), "an object id, not the pid");
    }
}
