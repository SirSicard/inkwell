//! The far end: a Core Audio process tap inside a private, **tap-only** aggregate device.
//!
//! Provenance: the sequence here (a `CATapDescription`, `AudioHardwareCreateProcessTap`, a private
//! aggregate device listing the tap, an IOProc on the aggregate) descends from AudioCap by
//! Guilherme Rambo, BSD-2-Clause, <https://github.com/insidegui/AudioCap>, by way of an earlier
//! implementation. Rewritten in Rust for this crate; see THIRD_PARTY.md.
//!
//! Decisions, each from a measurement on a real machine:
//!
//! - **Tap-only.** The aggregate holds the tap and no microphone. A Bluetooth mic placed inside an
//!   aggregate delivers digital zeros while the same mic works on its own, so the mic always runs
//!   on its own graph ([`MacMicSource`](super::MacMicSource), an IOProc on the device itself) and
//!   the two streams are aligned by host time. The aggregate is clocked by the default output
//!   device, which is what the tap follows.
//! - **No callbacks while nothing plays.** A tap-only aggregate is driven by the tap, so it calls
//!   back only while some tapped process makes a sound. Zero callbacks on the far end is idle, not
//!   broken ([`CaptureHealth::Idle`](super::CaptureHealth::Idle)).
//! - **Process object ids, never pids.** A tap names processes by their HAL *process object* id.
//!   A pid is the same width, so passing one compiles, runs, and taps (or excludes) some unrelated
//!   object. [`process_objects_for_apps`] maps apps (bundle ids, and pids when known) to process
//!   objects.
//! - **A denied tap is silent, not an error.** Without the System Audio permission the tap
//!   delivers zeros. Opening and starting succeed; the permission probe (a self-tap tone) is how
//!   the app finds out.
#![cfg(target_os = "macos")]

use std::collections::BTreeSet;
use std::ptr::NonNull;
use std::sync::Arc;

use ink_audio::RealtimeGuard;
use ink_core::{
    AppRef, AudioSink, AudioSource, Channel, FarEndTarget, PlatformError, SourceStats, StreamFormat,
};
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2_core_audio::{
    AudioHardwareCreateAggregateDevice, AudioHardwareCreateProcessTap,
    AudioHardwareDestroyAggregateDevice, AudioHardwareDestroyProcessTap, CATapDescription,
    CATapMuteBehavior, kAudioAggregateDeviceIsPrivateKey, kAudioAggregateDeviceIsStackedKey,
    kAudioAggregateDeviceMainSubDeviceKey, kAudioAggregateDeviceNameKey,
    kAudioAggregateDeviceSubDeviceListKey, kAudioAggregateDeviceTapAutoStartKey,
    kAudioAggregateDeviceTapListKey, kAudioAggregateDeviceUIDKey, kAudioDevicePropertyDeviceUID,
    kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyScopeInput,
    kAudioSubTapDriftCompensationKey, kAudioSubTapUIDKey, kAudioTapPropertyFormat,
    kAudioTapPropertyUID,
};
use objc2_core_foundation::{
    CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType,
};
use objc2_foundation::{NSArray, NSNumber, NSString, NSUUID};

use super::hal::{
    self, Direction, HalError, ObjectId, ProcessHal, UNKNOWN, belongs_to_app, capture_format, check,
};
use super::io::{InputContext, IoCounters, IoStats, RunningIo, finish, input_proc};
use crate::clock::MacClock;

/// The prefix of every aggregate device this crate creates. Private aggregates are visible to the
/// process that made them, so device listings filter this prefix out.
pub(crate) const AGGREGATE_UID_PREFIX: &str = "inkwell.capture.";

/// What a tap takes in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TapScope {
    /// Everything every process plays, except these process objects (our own, so the app's own
    /// sounds never reach the far end).
    AllExcept(Vec<ObjectId>),
    /// Only these process objects.
    Only(Vec<ObjectId>),
}

/// The process objects of `apps`: each app's known pid, translated, plus every process whose
/// bundle id is the app's or a helper under it. Sorted and without duplicates.
///
/// Audio an app hands to a shared system process is not included (a browser engine's media
/// process with a system bundle id, say). That is why the default far end is
/// [`FarEndTarget::AllOutput`].
pub(crate) fn process_objects_for_apps(
    hal: &dyn ProcessHal,
    apps: &[AppRef],
) -> Result<Vec<ObjectId>, HalError> {
    let mut found = BTreeSet::new();
    for pid in apps.iter().filter_map(|a| a.pid) {
        let Ok(pid) = i32::try_from(pid) else {
            continue;
        };
        if let Some(object) = hal.process_object_for_pid(pid)? {
            found.insert(object);
        }
    }
    for object in hal.process_objects()? {
        let bundle = match hal.bundle_id(object) {
            Ok(bundle) => bundle,
            Err(e) if e.is_gone() => continue,
            Err(e) => return Err(e),
        };
        if apps.iter().any(|app| belongs_to_app(&bundle, &app.id)) {
            found.insert(object);
        }
    }
    Ok(found.into_iter().collect())
}

/// What the far end taps for `target`. `own_pid` is this process, excluded from a global tap.
pub(crate) fn tap_scope(
    hal: &dyn ProcessHal,
    target: &FarEndTarget,
    own_pid: i32,
) -> Result<TapScope, PlatformError> {
    match target {
        // If this process is not yet a client of the audio server it has no process object, and
        // it plays nothing, so there is nothing to exclude.
        FarEndTarget::AllOutput => Ok(TapScope::AllExcept(
            hal.process_object_for_pid(own_pid)?.into_iter().collect(),
        )),
        FarEndTarget::Apps(apps) => {
            let objects = process_objects_for_apps(hal, apps)?;
            if objects.is_empty() {
                // Never a tap of nothing: that would be a silent far end that looks like a quiet
                // meeting.
                return Err(PlatformError::Device(format!(
                    "none of the {} requested apps has an audio process to tap",
                    apps.len()
                )));
            }
            Ok(TapScope::Only(objects))
        }
    }
}

/// The tap description for `scope`: mono, private, with `mute` as its mute behaviour.
pub(crate) fn tap_description(
    scope: &TapScope,
    mute: CATapMuteBehavior,
) -> Retained<CATapDescription> {
    let ids = match scope {
        TapScope::AllExcept(ids) | TapScope::Only(ids) => ids,
    };
    let numbers: Vec<Retained<NSNumber>> = ids.iter().map(|&id| NSNumber::new_u32(id)).collect();
    let processes = NSArray::from_retained_slice(&numbers);
    let alloc = CATapDescription::alloc();
    // SAFETY: both initialisers take an NSArray of NSNumbers holding process object ids, which
    // `processes` is; they return an initialised description.
    let description = unsafe {
        match scope {
            TapScope::AllExcept(_) => {
                CATapDescription::initMonoGlobalTapButExcludeProcesses(alloc, &processes)
            }
            TapScope::Only(_) => CATapDescription::initMonoMixdownOfProcesses(alloc, &processes),
        }
    };
    // SAFETY: plain property setters on a live description; the strings and UUID are copied.
    unsafe {
        description.setName(&NSString::from_str("Inkwell far end"));
        description.setUUID(&NSUUID::new());
        // Private: no other process sees the tap.
        description.setPrivate(true);
        description.setMuteBehavior(mute);
    }
    description
}

/// The spec of the far end's aggregate device. There is no field for sub-devices: the aggregate
/// is tap-only by construction (see the module docs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AggregateSpec {
    /// The aggregate's UID, unique per open.
    pub uid: String,
    /// The device whose clock drives the aggregate: the default output.
    pub clock_device_uid: String,
    /// The tap it carries.
    pub tap_uid: String,
}

impl AggregateSpec {
    pub(crate) fn tap_only(clock_device_uid: String, tap_uid: String) -> Self {
        // SAFETY: -[NSUUID UUIDString] on a fresh UUID; returns a string.
        let unique = NSUUID::new().UUIDString().to_string();
        Self {
            uid: format!("{AGGREGATE_UID_PREFIX}{unique}"),
            clock_device_uid,
            tap_uid,
        }
    }

    /// The description `AudioHardwareCreateAggregateDevice` takes.
    pub(crate) fn to_dictionary(&self) -> CFRetained<CFDictionary<CFString, CFType>> {
        let key = |k: &std::ffi::CStr| CFString::from_str(&k.to_string_lossy());
        let tap = CFDictionary::<CFString, CFType>::from_slices(
            &[
                &*key(kAudioSubTapUIDKey),
                &*key(kAudioSubTapDriftCompensationKey),
            ],
            &[&CFString::from_str(&self.tap_uid), &CFNumber::new_i32(1)],
        );
        let taps = CFArray::from_retained_objects(&[tap]);
        let no_sub_devices = CFArray::<CFType>::empty();
        let name = CFString::from_str("Inkwell far end");
        let uid = CFString::from_str(&self.uid);
        let clock = CFString::from_str(&self.clock_device_uid);
        let keys = [
            key(kAudioAggregateDeviceNameKey),
            key(kAudioAggregateDeviceUIDKey),
            key(kAudioAggregateDeviceMainSubDeviceKey),
            key(kAudioAggregateDeviceIsPrivateKey),
            key(kAudioAggregateDeviceIsStackedKey),
            key(kAudioAggregateDeviceTapAutoStartKey),
            key(kAudioAggregateDeviceSubDeviceListKey),
            key(kAudioAggregateDeviceTapListKey),
        ];
        let values: [&CFType; 8] = [
            &name,
            &uid,
            &clock,
            CFBoolean::new(true),
            CFBoolean::new(false),
            CFBoolean::new(true),
            &no_sub_devices,
            &taps,
        ];
        let keys: Vec<&CFString> = keys.iter().map(|k| &**k).collect();
        CFDictionary::from_slices(&keys, &values)
    }
}

/// A process tap; destroyed on drop.
struct ProcessTap(ObjectId);

impl Drop for ProcessTap {
    fn drop(&mut self) {
        // SAFETY: the id came from `AudioHardwareCreateProcessTap` and is destroyed once. A
        // failure leaves nothing to do: the server drops the tap with this process anyway.
        unsafe { AudioHardwareDestroyProcessTap(self.0) };
    }
}

/// An aggregate device; destroyed on drop.
struct Aggregate(ObjectId);

impl Drop for Aggregate {
    fn drop(&mut self) {
        // SAFETY: the id came from `AudioHardwareCreateAggregateDevice` and is destroyed once. A
        // private aggregate also goes away with this process.
        unsafe { AudioHardwareDestroyAggregateDevice(self.0) };
    }
}

/// The far-end [`AudioSource`]: a process tap in a tap-only aggregate device.
///
/// **No callbacks while nothing plays** (the aggregate is driven by the tap), so zero callbacks
/// is idle, and host times jump across the silence. **A denied tap delivers zeros**: check
/// `PermissionProbe::check(SystemAudio)` before a meeting.
pub struct MacFarEndSource {
    // Field order is drop order: IO stops, then the aggregate goes, then the tap.
    running: Option<RunningIo<InputContext>>,
    aggregate: Aggregate,
    #[expect(
        dead_code,
        reason = "held for its Drop, which destroys the tap after the aggregate"
    )]
    tap: ProcessTap,
    format: StreamFormat,
    clock: MacClock,
    guard: RealtimeGuard,
    counters: Arc<IoCounters>,
    tapped: usize,
}

impl MacFarEndSource {
    /// Creates the tap and its aggregate for `scope`. IO starts in `start`.
    pub(crate) fn open(
        scope: TapScope,
        mute: CATapMuteBehavior,
        clock: MacClock,
        guard: RealtimeGuard,
    ) -> Result<Self, PlatformError> {
        let tapped = match &scope {
            TapScope::AllExcept(_) => 0,
            TapScope::Only(ids) => ids.len(),
        };
        let description = tap_description(&scope, mute);
        let mut tap_id = UNKNOWN;
        // SAFETY: a live description and a live out-parameter.
        let status = unsafe { AudioHardwareCreateProcessTap(Some(&description), &mut tap_id) };
        check(status, "creating the system audio tap")?;
        let tap = ProcessTap(tap_id);
        let tap_uid = hal::get_string(
            tap.0,
            kAudioTapPropertyUID,
            kAudioObjectPropertyScopeGlobal,
            "reading the tap's UID",
        )?;
        let output = hal::default_device(Direction::Output)?.ok_or_else(|| {
            PlatformError::Device("no default output device to clock the far end".into())
        })?;
        let output_uid = hal::get_string(
            output,
            kAudioDevicePropertyDeviceUID,
            kAudioObjectPropertyScopeGlobal,
            "reading the output device's UID",
        )?;
        let spec = AggregateSpec::tap_only(output_uid, tap_uid);
        let dictionary = spec.to_dictionary();
        let mut aggregate_id = UNKNOWN;
        // SAFETY: a well-formed aggregate description (CFString keys, CF values of the documented
        // types) and a live out-parameter.
        let status = unsafe {
            AudioHardwareCreateAggregateDevice(
                (*dictionary).as_ref(),
                NonNull::from(&mut aggregate_id),
            )
        };
        check(status, "creating the far end's aggregate device")?;
        let aggregate = Aggregate(aggregate_id);
        // The aggregate's own input stream is what the IOProc receives, so its format is the one
        // the blocks are labelled with. If the aggregate does not list the stream (yet), the tap's
        // format stands in; the IOProc still refuses any buffer whose channel count differs
        // (counted as skipped), and the pump's rate check catches a wrong rate.
        let asbd = match hal::first_stream_format(aggregate.0, kAudioObjectPropertyScopeInput)? {
            Some(asbd) => asbd,
            None => hal::get(
                tap.0,
                kAudioTapPropertyFormat,
                kAudioObjectPropertyScopeGlobal,
                "reading the tap's format",
            )?,
        };
        let format = capture_format(&asbd)
            .map_err(|problem| PlatformError::Device(format!("system audio tap: {problem}")))?;
        Ok(Self {
            running: None,
            aggregate,
            tap,
            format,
            clock,
            guard,
            counters: Arc::default(),
            tapped,
        })
    }

    /// The counters so far. **Any thread** that holds the source.
    pub fn stats(&self) -> IoStats {
        self.counters.snapshot()
    }

    /// How many process objects an app tap covers; 0 for the global tap.
    pub fn tapped_processes(&self) -> usize {
        self.tapped
    }
}

impl AudioSource for MacFarEndSource {
    fn channel(&self) -> Channel {
        Channel::Far
    }

    fn format(&self) -> StreamFormat {
        self.format
    }

    fn start(&mut self, sink: Box<dyn AudioSink>) -> Result<(), PlatformError> {
        if self.running.is_some() {
            return Err(PlatformError::Failed(
                "the far end is already started".into(),
            ));
        }
        let context = InputContext::new(
            sink,
            self.guard.clone(),
            self.format,
            self.clock,
            Arc::clone(&self.counters),
        );
        match RunningIo::start(self.aggregate.0, Some(input_proc), context) {
            Ok(running) => {
                self.running = Some(running);
                Ok(())
            }
            // The sink is dropped with its context, so the ring's consumer sees it abandoned.
            Err((_context, error)) => Err(error.into()),
        }
    }

    fn stop(&mut self) -> Result<SourceStats, PlatformError> {
        let Some(running) = self.running.take() else {
            return Ok(SourceStats::default());
        };
        finish(running, &self.counters, "far end")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::hal::fake::FakeHal;

    fn app(id: &str, pid: Option<u32>) -> AppRef {
        AppRef {
            id: id.into(),
            pid,
            name: id.into(),
        }
    }

    /// Taps by app take HAL process-object ids, not pids. The fake server's object ids are
    /// deliberately far from its pids, so a pid passed through would show.
    #[test]
    fn app_taps_take_hal_process_object_ids_not_pids() {
        let hal = FakeHal::with(&[
            (71, 5_001, "com.example.Meeting", true),
            (72, 5_002, "com.example.Meeting.helper", false),
            (73, 5_003, "com.example.Other", false),
        ]);
        let apps = [app("com.example.Meeting", Some(5_001))];
        let objects = process_objects_for_apps(&hal, &apps).unwrap();
        assert_eq!(
            objects,
            vec![71, 72],
            "the app and its helper, as process objects"
        );
        assert!(!objects.contains(&5_001), "never the pid");

        let scope = tap_scope(&hal, &FarEndTarget::Apps(apps.to_vec()), 1).unwrap();
        assert_eq!(scope, TapScope::Only(vec![71, 72]));
        // And the description handed to the HAL carries those ids.
        let description = tap_description(&scope, CATapMuteBehavior::Unmuted);
        // SAFETY: a property getter on a live description.
        let processes = unsafe { description.processes() };
        let ids: Vec<u32> = (0..processes.count())
            .map(|i| processes.objectAtIndex(i).as_u32())
            .collect();
        assert_eq!(ids, vec![71, 72]);
        // SAFETY: as above.
        unsafe {
            assert!(description.isPrivate());
            assert!(description.isMono());
            assert!(!description.isExclusive(), "an include list");
        }
    }

    #[test]
    fn an_app_known_only_by_pid_is_found_through_the_translation() {
        let hal = FakeHal::with(&[(90, 777, "", true)]);
        let objects = process_objects_for_apps(&hal, &[app("tool", Some(777))]).unwrap();
        assert_eq!(objects, vec![90]);
    }

    #[test]
    fn an_app_with_no_audio_process_is_an_error_not_a_silent_tap() {
        let hal = FakeHal::with(&[(71, 5_001, "com.example.Other", true)]);
        let result = tap_scope(
            &hal,
            &FarEndTarget::Apps(vec![app("com.example.Meeting", None)]),
            1,
        );
        assert!(
            matches!(result, Err(PlatformError::Device(_))),
            "{result:?}"
        );
    }

    #[test]
    fn the_global_tap_excludes_our_own_process_object() {
        let hal = FakeHal::with(&[(40, 4_242, "", true), (41, 9, "com.example.Meeting", true)]);
        let scope = tap_scope(&hal, &FarEndTarget::AllOutput, 4_242).unwrap();
        assert_eq!(scope, TapScope::AllExcept(vec![40]));
        let description = tap_description(&scope, CATapMuteBehavior::Unmuted);
        // SAFETY: property getters on a live description.
        unsafe {
            assert!(description.isExclusive(), "an exclude list");
            assert_eq!(description.isMuted(), CATapMuteBehavior::Unmuted);
        }
        // Not yet a client of the audio server: nothing to exclude.
        let scope = tap_scope(&hal, &FarEndTarget::AllOutput, 1).unwrap();
        assert_eq!(scope, TapScope::AllExcept(vec![]));
    }

    /// The far end's aggregate is tap-only: a Bluetooth mic inside an aggregate delivers digital
    /// zeros, so no microphone ever goes into one (the dual graph).
    #[test]
    fn the_far_end_aggregate_is_tap_only_so_no_mic_ever_sits_inside_it() {
        let spec = AggregateSpec::tap_only("output-uid".into(), "tap-uid".into());
        assert!(spec.uid.starts_with(AGGREGATE_UID_PREFIX));
        let dictionary = spec.to_dictionary();
        let get = |key: &std::ffi::CStr| {
            dictionary
                .get(&CFString::from_str(&key.to_string_lossy()))
                .unwrap_or_else(|| panic!("{key:?} missing"))
        };
        let sub_devices = get(kAudioAggregateDeviceSubDeviceListKey)
            .downcast::<CFArray>()
            .expect("an array");
        assert_eq!(sub_devices.len(), 0, "no sub-devices: no microphone");
        let taps = get(kAudioAggregateDeviceTapListKey)
            .downcast::<CFArray>()
            .expect("an array");
        assert_eq!(taps.len(), 1);
        let main = get(kAudioAggregateDeviceMainSubDeviceKey)
            .downcast::<CFString>()
            .expect("a string");
        assert_eq!(main.to_string(), "output-uid", "clocked by the output");
        let private = get(kAudioAggregateDeviceIsPrivateKey)
            .downcast::<CFBoolean>()
            .expect("a boolean");
        assert!(private.as_bool());
        let auto_start = get(kAudioAggregateDeviceTapAutoStartKey)
            .downcast::<CFBoolean>()
            .expect("a boolean");
        assert!(auto_start.as_bool());
    }

    #[test]
    fn every_aggregate_gets_its_own_uid() {
        let a = AggregateSpec::tap_only("o".into(), "t".into());
        let b = AggregateSpec::tap_only("o".into(), "t".into());
        assert_ne!(a.uid, b.uid);
    }
}
