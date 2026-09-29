//! A WASAPI capture stream on its own thread, and the only code in this crate that runs in the
//! realtime zone.
//!
//! One type serves all three captures: the mic (a capture endpoint), device loopback (a render
//! endpoint, everything it plays) and process loopback (one process tree, through the
//! `VAD\Process_Loopback` virtual device). Each runs on a thread of its own, registered with MMCSS
//! as "Pro Audio", which waits on the stream's event (or a 10 ms poll for device loopback, whose
//! event is not relied on) and drains every packet waiting.
//!
//! **Realtime zone.** One wake's drain ([`drain`]) runs inside the [`RealtimeGuard`]: it takes
//! packets from the capture client, hands each to the sink (`ink-audio`'s ring producer, which
//! copies and returns) and releases it. It does not allocate, lock, log or block; a silent packet
//! is pushed as zeros from a buffer allocated when the stream starts. Tests run it under
//! `assert_no_alloc` with a scripted packet source (I4); the checklist binary does the same on real
//! devices. The wait between wakes is outside the guard: it is the equivalent of the HAL calling
//! an IOProc.
//!
//! **What a packet says.** `AUDCLNT_BUFFERFLAGS_SILENT` means the data must be read as zeros, so
//! zeros are pushed (never the buffer, which may hold anything). `DATA_DISCONTINUITY` after the
//! first packet is a gap. On the mic, a jump in the device position is a gap too; on loopback it
//! is not, because loopback delivers nothing while nothing plays and resumes at a later position
//! (idle, not lost). `TIMESTAMP_ERROR`, or no stamp, is stamped with the clock's time instead.
//!
//! **The stream's end.** An error from the capture client (the device was unplugged, or its
//! format changed: `AUDCLNT_E_DEVICE_INVALIDATED`) ends delivery; `stop` reports it. So does a
//! panic, which `catch_unwind` stops at the thread's edge. Reopening at the new device mid-session
//! is the device-change work of the meeting chain (S3.5b), as on the Mac.
#![cfg(windows)]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ink_audio::RealtimeGuard;
use ink_core::{AudioBlock, AudioSink, Clock, PlatformError, SourceStats, StreamFormat};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY, AUDCLNT_BUFFERFLAGS_SILENT,
    AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioCaptureClient,
    IAudioClient, WAVEFORMATEX, WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0,
};
use windows::Win32::Media::KernelStreaming::WAVE_FORMAT_EXTENSIBLE;
use windows::Win32::Media::Multimedia::KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
use windows::Win32::System::Com::{CLSCTX_ALL, CoTaskMemFree};
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW, SetEvent,
    WaitForMultipleObjects,
};
use windows::core::{HRESULT, w};

use super::devices;
use super::loopback;
use crate::clock::{WinClock, qpc_position_to_ns};
use crate::com::{ComScope, device_error};

/// The shared-mode buffer asked for: 100 ms. The engine's period (about 10 ms) sets the wake rate;
/// this is how much a late wake can catch up on before the engine drops audio.
const BUFFER_HNS: i64 = 1_000_000;

/// Frames of zeros pushed per block for a silent packet. Any packet size works: a larger one is
/// pushed as several blocks.
const ZERO_FRAMES: usize = 1_024;

/// How long `start` waits for the stream to come up (process loopback activates asynchronously).
const START_TIMEOUT: Duration = Duration::from_secs(5);

/// The format process loopback is opened in. It has no mix format of its own; the engine
/// converts into whatever is asked (S0.4's probe heard a browser this way).
pub(crate) const PROCESS_LOOPBACK_FORMAT: StreamFormat = StreamFormat {
    sample_rate: 48_000,
    channels: 2,
};

/// What a stream captures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StreamKind {
    /// A capture endpoint: a microphone.
    Mic {
        /// The endpoint id.
        endpoint: String,
    },
    /// Everything a render endpoint plays.
    DeviceLoopback {
        /// The endpoint id.
        endpoint: String,
    },
    /// One process and its child processes, whatever device they play to.
    ProcessLoopback {
        /// The root of the tree.
        pid: u32,
    },
}

impl StreamKind {
    /// Whether packets keep coming when there is nothing to hear. Only the mic: loopback goes
    /// quiet while nothing plays.
    fn continuous(&self) -> bool {
        matches!(self, Self::Mic { .. })
    }

    /// Whether the stream's event is waited on. Device loopback polls, as S0.4's probe did, so it
    /// never depends on loopback event signalling.
    fn event_driven(&self) -> bool {
        !matches!(self, Self::DeviceLoopback { .. })
    }
}

/// A stream's counters so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IoStats {
    /// Wakes that found at least one packet. Zero on loopback while nothing plays; zero on a
    /// started mic means a stalled device.
    pub callbacks: u64,
    /// Packets delivered.
    pub packets: u64,
    /// Of those, packets flagged silent (pushed as zeros).
    pub silent_packets: u64,
    /// Frames handed to the sink.
    pub frames: u64,
    /// Gaps: `DATA_DISCONTINUITY` after the first packet, and on the mic, device-position jumps.
    pub discontinuities: u64,
    /// Packets without a usable timestamp, stamped with the clock's time instead.
    pub untimed: u64,
    /// Packets not delivered because their buffer was unusable (null or misaligned). Each is a gap.
    pub skipped: u64,
    /// Panics caught on the capture thread. After one, the stream stops.
    pub panics: u64,
    /// Whether the thread runs under MMCSS ("Pro Audio"). Without it capture still works, at
    /// normal priority.
    pub mmcss: bool,
    /// The error that ended delivery, as an HRESULT; 0 while none has.
    pub ended_by: i32,
}

/// The live counters behind [`IoStats`].
#[derive(Debug, Default)]
pub(crate) struct Counters {
    callbacks: AtomicU64,
    packets: AtomicU64,
    silent_packets: AtomicU64,
    frames: AtomicU64,
    discontinuities: AtomicU64,
    untimed: AtomicU64,
    skipped: AtomicU64,
    panics: AtomicU64,
    mmcss: AtomicBool,
    ended_by: AtomicI32,
}

impl Counters {
    pub(crate) fn snapshot(&self) -> IoStats {
        let load = |a: &AtomicU64| a.load(Ordering::Relaxed);
        IoStats {
            callbacks: load(&self.callbacks),
            packets: load(&self.packets),
            silent_packets: load(&self.silent_packets),
            frames: load(&self.frames),
            discontinuities: load(&self.discontinuities),
            untimed: load(&self.untimed),
            skipped: load(&self.skipped),
            panics: load(&self.panics),
            mmcss: self.mmcss.load(Ordering::Relaxed),
            ended_by: self.ended_by.load(Ordering::Relaxed),
        }
    }
}

/// One packet as the capture client hands it over.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RawPacket {
    pub(crate) data: *const u8,
    pub(crate) frames: u32,
    pub(crate) flags: u32,
    pub(crate) device_position: u64,
    /// The performance counter at the first frame, in 100-ns units; 0 when not given.
    pub(crate) qpc_100ns: u64,
}

/// Where packets come from: the capture client, or a script in the tests.
///
/// **Realtime.** Every method runs inside the guard.
pub(crate) trait PacketSource {
    /// Frames in the next packet; 0 when none is waiting.
    fn next_size(&mut self) -> Result<u32, HRESULT>;
    /// The next packet. Its data is valid until [`release`](Self::release).
    fn get(&mut self) -> Result<RawPacket, HRESULT>;
    /// Gives the packet back.
    fn release(&mut self, frames: u32) -> Result<(), HRESULT>;
}

/// [`PacketSource`] on `IAudioCaptureClient`.
struct ClientPackets(IAudioCaptureClient);

impl PacketSource for ClientPackets {
    fn next_size(&mut self) -> Result<u32, HRESULT> {
        // SAFETY: a live capture client on its own thread.
        unsafe { self.0.GetNextPacketSize() }.map_err(|e| e.code())
    }

    fn get(&mut self) -> Result<RawPacket, HRESULT> {
        let mut data = std::ptr::null_mut();
        let mut frames = 0u32;
        let mut flags = 0u32;
        let mut device_position = 0u64;
        let mut qpc_100ns = 0u64;
        // SAFETY: every out-pointer is a live local for the call.
        unsafe {
            self.0.GetBuffer(
                &mut data,
                &mut frames,
                &mut flags,
                Some(&mut device_position),
                Some(&mut qpc_100ns),
            )
        }
        .map_err(|e| e.code())?;
        Ok(RawPacket {
            data,
            frames,
            flags,
            device_position,
            qpc_100ns,
        })
    }

    fn release(&mut self, frames: u32) -> Result<(), HRESULT> {
        // SAFETY: releases the packet `get` returned, with its frame count.
        unsafe { self.0.ReleaseBuffer(frames) }.map_err(|e| e.code())
    }
}

/// The realtime side of a stream: the sink and what it needs to stamp blocks.
pub(crate) struct Delivery {
    sink: Box<dyn AudioSink>,
    format: StreamFormat,
    clock: WinClock,
    continuous: bool,
    zeros: Box<[f32]>,
    counters: Arc<Counters>,
    /// The device position the next packet should start at; `None` before the first.
    next_position: Option<u64>,
    first: bool,
}

impl Delivery {
    pub(crate) fn new(
        sink: Box<dyn AudioSink>,
        format: StreamFormat,
        clock: WinClock,
        continuous: bool,
        counters: Arc<Counters>,
    ) -> Self {
        let channels = usize::from(format.channels.max(1));
        Self {
            sink,
            format,
            clock,
            continuous,
            zeros: vec![0.0; ZERO_FRAMES * channels].into_boxed_slice(),
            counters,
            next_position: None,
            first: true,
        }
    }

    /// The sink back, to drop on the owner's thread.
    pub(crate) fn into_sink(self) -> Box<dyn AudioSink> {
        self.sink
    }

    /// **Realtime.** One packet.
    fn packet(&mut self, packet: &RawPacket) {
        let c = &self.counters;
        c.packets.fetch_add(1, Ordering::Relaxed);
        let frames = packet.frames as usize;
        let flag = |f: i32| packet.flags & (f as u32) != 0;
        let mut gap = !self.first && flag(AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0);
        if self.continuous
            && self
                .next_position
                .is_some_and(|expected| expected != packet.device_position)
        {
            gap = true;
        }
        if gap {
            c.discontinuities.fetch_add(1, Ordering::Relaxed);
        }
        self.first = false;
        self.next_position = Some(packet.device_position.wrapping_add(packet.frames.into()));
        if frames == 0 {
            return;
        }
        let host_time_ns = if packet.qpc_100ns == 0 || flag(AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0) {
            c.untimed.fetch_add(1, Ordering::Relaxed);
            self.clock.now_ns()
        } else {
            qpc_position_to_ns(packet.qpc_100ns)
        };
        let channels = usize::from(self.format.channels.max(1));
        if flag(AUDCLNT_BUFFERFLAGS_SILENT.0) {
            c.silent_packets.fetch_add(1, Ordering::Relaxed);
            push_zeros(
                &mut *self.sink,
                &self.zeros,
                self.format,
                frames,
                host_time_ns,
            );
        } else {
            let data = packet.data.cast::<f32>();
            if data.is_null() || !data.is_aligned() {
                c.skipped.fetch_add(1, Ordering::Relaxed);
                // The next packet cannot be checked against this one.
                self.next_position = None;
                return;
            }
            // SAFETY: the capture client's buffer holds `frames` frames of the float format the
            // stream was initialised with, valid until it is released after this call; the
            // pointer is non-null and aligned (checked).
            let samples = unsafe { std::slice::from_raw_parts(data, frames * channels) };
            self.sink.push(&AudioBlock {
                samples,
                format: self.format,
                host_time_ns,
            });
        }
        c.frames.fetch_add(frames as u64, Ordering::Relaxed);
    }
}

/// **Realtime.** `frames` of silence, in blocks of the preallocated `zeros`.
fn push_zeros(
    sink: &mut dyn AudioSink,
    zeros: &[f32],
    format: StreamFormat,
    frames: usize,
    host_time_ns: u64,
) {
    let channels = usize::from(format.channels.max(1));
    let block = zeros.len() / channels;
    let rate = u64::from(format.sample_rate.max(1));
    let mut done = 0usize;
    while done < frames && block > 0 {
        let n = (frames - done).min(block);
        let offset_ns = done as u64 * 1_000_000_000 / rate;
        sink.push(&AudioBlock {
            samples: &zeros[..n * channels],
            format,
            host_time_ns: host_time_ns.saturating_add(offset_ns),
        });
        done += n;
    }
}

/// **Realtime.** Takes every waiting packet, delivers it and releases it. An error from the
/// capture client ends the stream: it is returned, and the caller stops waking.
pub(crate) fn drain(source: &mut dyn PacketSource, delivery: &mut Delivery) -> Result<(), HRESULT> {
    let mut any = false;
    loop {
        if source.next_size()? == 0 {
            break;
        }
        let packet = source.get()?;
        if !any {
            any = true;
            delivery.counters.callbacks.fetch_add(1, Ordering::Relaxed);
        }
        delivery.packet(&packet);
        source.release(packet.frames)?;
    }
    Ok(())
}

/// One wake, inside the guard, with a panic stopped at the thread's edge. `false` once the stream
/// must stop (an error or a panic, both counted).
pub(crate) fn wake(
    guard: &RealtimeGuard,
    source: &mut dyn PacketSource,
    delivery: &mut Delivery,
) -> bool {
    let mut result = Ok(());
    let caught = catch_unwind(AssertUnwindSafe(|| {
        guard(&mut || result = drain(source, delivery));
    }));
    let counters = &delivery.counters;
    if caught.is_err() {
        counters.panics.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    match result {
        Ok(()) => true,
        Err(hr) => {
            // A zero HRESULT is success; an error code is never zero.
            counters.ended_by.store(hr.0, Ordering::Relaxed);
            false
        }
    }
}

/// A Win32 event, closed on drop.
pub(crate) struct Event(HANDLE);

// SAFETY: an event handle may be waited on, set and closed from any thread.
unsafe impl Send for Event {}
// SAFETY: as above; `SetEvent` and waits are thread-safe.
unsafe impl Sync for Event {}

impl Event {
    /// An auto-reset event, not set.
    pub(crate) fn new() -> Result<Self, PlatformError> {
        // SAFETY: default security, unnamed.
        unsafe { CreateEventW(None, false, false, None) }
            .map(Self)
            .map_err(|e| PlatformError::Failed(format!("creating an event: {e}")))
    }

    pub(crate) fn handle(&self) -> HANDLE {
        self.0
    }

    pub(crate) fn set(&self) {
        // SAFETY: a live event handle.
        let _ = unsafe { SetEvent(self.0) };
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: ours, closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// The speaker mask for a channel count when the device gives none: mono is front centre, stereo
/// front left and right, more are left unassigned.
pub(crate) fn default_mask(channels: u16) -> u32 {
    match channels {
        1 => 0x4,
        2 => 0x3,
        _ => 0,
    }
}

/// A float `WAVEFORMATEXTENSIBLE` of `format`, with the device's speaker `mask` (a 5.1 or 7.1
/// output is refused without its real layout).
fn float_format(format: StreamFormat, mask: u32) -> WAVEFORMATEXTENSIBLE {
    let channels = format.channels.max(1);
    let block_align = channels * 4;
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_EXTENSIBLE as u16,
            nChannels: channels,
            nSamplesPerSec: format.sample_rate,
            nAvgBytesPerSec: format.sample_rate * u32::from(block_align),
            nBlockAlign: block_align,
            wBitsPerSample: 32,
            cbSize: (size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>()) as u16,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 {
            wValidBitsPerSample: 32,
        },
        dwChannelMask: mask,
        SubFormat: KSDATAFORMAT_SUBTYPE_IEEE_FLOAT,
    }
}

/// The shared-mode mix format of `client`, as the core's [`StreamFormat`] and its speaker mask.
pub(crate) fn mix_format(client: &IAudioClient) -> Result<(StreamFormat, u32), PlatformError> {
    // SAFETY: a live client; the returned block is freed below.
    let raw = unsafe { client.GetMixFormat() }
        .map_err(|e| device_error("reading the device's format", &e))?;
    if raw.is_null() {
        return Err(PlatformError::Device(
            "the device reported no format".into(),
        ));
    }
    // SAFETY: GetMixFormat returns a valid WAVEFORMATEX (packed; read by value), and a
    // WAVEFORMATEXTENSIBLE when its tag says so and its extension is long enough.
    let (rate, channels, mask) = unsafe {
        let header = *raw;
        let extensible = u32::from(header.wFormatTag) == WAVE_FORMAT_EXTENSIBLE
            && usize::from(header.cbSize)
                >= size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>();
        let mask = if extensible {
            (*raw.cast::<WAVEFORMATEXTENSIBLE>()).dwChannelMask
        } else {
            default_mask(header.nChannels)
        };
        (header.nSamplesPerSec, header.nChannels, mask)
    };
    // SAFETY: allocated by GetMixFormat with CoTaskMemAlloc, freed once.
    unsafe { CoTaskMemFree(Some(raw.cast())) };
    if rate == 0 || channels == 0 {
        return Err(PlatformError::Device(format!(
            "the device reported {rate} Hz and {channels} channels"
        )));
    }
    Ok((
        StreamFormat {
            sample_rate: rate,
            channels,
        },
        mask,
    ))
}

/// The layout to open a device in: its own mix format when the engine takes it, else stereo at the
/// same rate. Up to two channels are taken as they are; more are tried with `accepts` (an
/// `Initialize` on a fresh client), because a multichannel output (5.1 HDMI) can refuse a float
/// stream in its own layout. Pure over `accepts`.
pub(crate) fn choose_layout(
    mix: (StreamFormat, u32),
    mut accepts: impl FnMut(StreamFormat, u32) -> bool,
) -> Option<(StreamFormat, u32)> {
    if mix.0.channels <= 2 || accepts(mix.0, mix.1) {
        return Some(mix);
    }
    let stereo = StreamFormat {
        sample_rate: mix.0.sample_rate,
        channels: 2,
    };
    accepts(stereo, default_mask(2)).then_some((stereo, default_mask(2)))
}

/// The stream flags for `kind`.
fn stream_flags(kind: &StreamKind) -> u32 {
    let mut flags = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    if !matches!(kind, StreamKind::Mic { .. }) {
        flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
    }
    if kind.event_driven() {
        flags |= AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
    }
    flags
}

/// `Initialize` in shared mode, float, in `format` with speaker `mask`.
fn initialize(
    client: &IAudioClient,
    kind: &StreamKind,
    format: StreamFormat,
    mask: u32,
) -> windows::core::Result<()> {
    let wave = float_format(format, mask);
    // SAFETY: `wave` is a complete WAVEFORMATEXTENSIBLE whose header it starts with, live for the
    // call.
    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            stream_flags(kind),
            BUFFER_HNS,
            0,
            std::ptr::from_ref(&wave).cast::<WAVEFORMATEX>(),
            None,
        )
    }
}

/// The format a stream of `kind` delivers and its speaker mask, read without starting it.
/// **Worker**, in a COM scope.
pub(crate) fn probe_format(kind: &StreamKind) -> Result<(StreamFormat, u32), PlatformError> {
    match kind {
        StreamKind::Mic { endpoint } | StreamKind::DeviceLoopback { endpoint } => {
            let devices = devices::enumerator()?;
            let device = devices::endpoint_by_id(&devices, endpoint)?;
            let activate = || -> Result<IAudioClient, PlatformError> {
                // SAFETY: a live device.
                unsafe { device.Activate(CLSCTX_ALL, None) }
                    .map_err(|e| device_error("opening the audio device", &e))
            };
            let mix = mix_format(&activate()?)?;
            // A client initialises once, so each try gets a fresh one; nothing is started.
            choose_layout(mix, |format, mask| {
                activate().is_ok_and(|client| initialize(&client, kind, format, mask).is_ok())
            })
            .ok_or_else(|| {
                PlatformError::Device(format!(
                    "the device takes neither its own {}-channel layout nor stereo",
                    mix.0.channels
                ))
            })
        }
        StreamKind::ProcessLoopback { .. } => Ok((PROCESS_LOOPBACK_FORMAT, default_mask(2))),
    }
}

/// Opens and initialises the client for `kind` in `format`, with the stream's event set when it
/// is event-driven. **The capture thread**, in its COM scope.
fn open_client(
    kind: &StreamKind,
    format: StreamFormat,
    mask: u32,
    event: &Event,
) -> Result<(IAudioClient, IAudioCaptureClient), PlatformError> {
    let client: IAudioClient = match kind {
        StreamKind::Mic { endpoint } | StreamKind::DeviceLoopback { endpoint } => {
            let devices = devices::enumerator()?;
            let device = devices::endpoint_by_id(&devices, endpoint)?;
            // SAFETY: a live device.
            unsafe { device.Activate(CLSCTX_ALL, None) }
                .map_err(|e| device_error("opening the audio device", &e))?
        }
        StreamKind::ProcessLoopback { pid } => loopback::activate(*pid)?,
    };
    initialize(&client, kind, format, mask)
        .map_err(|e| device_error("initialising the capture stream", &e))?;
    if kind.event_driven() {
        // SAFETY: a live event, kept alive by the stream for as long as the client.
        unsafe { client.SetEventHandle(event.handle()) }
            .map_err(|e| device_error("setting the capture event", &e))?;
    }
    // SAFETY: an initialised client.
    let capture: IAudioCaptureClient = unsafe { client.GetService() }
        .map_err(|e| device_error("opening the capture client", &e))?;
    Ok((client, capture))
}

/// Everything the capture thread needs.
struct ThreadSetup {
    kind: StreamKind,
    format: StreamFormat,
    mask: u32,
    delivery: Delivery,
    guard: RealtimeGuard,
    stop: Arc<Event>,
    counters: Arc<Counters>,
}

/// The capture thread: bring the stream up, say so, then wake and drain until stopped or ended.
/// Returns the delivery (with the sink) for the owner to drop.
fn run(setup: ThreadSetup, ready: mpsc::SyncSender<Result<(), PlatformError>>) -> Delivery {
    let ThreadSetup {
        kind,
        format,
        mask,
        mut delivery,
        guard,
        stop,
        counters,
    } = setup;
    let com = match ComScope::enter() {
        Ok(com) => com,
        Err(e) => {
            let _ = ready.send(Err(e));
            return delivery;
        }
    };
    let mut task_index = 0u32;
    // SAFETY: a static task name and a live out-parameter.
    let mmcss = unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task_index) }.ok();
    counters.mmcss.store(mmcss.is_some(), Ordering::Relaxed);
    let event = match Event::new() {
        Ok(event) => event,
        Err(e) => {
            let _ = ready.send(Err(e));
            return delivery;
        }
    };
    let (client, capture) = match open_client(&kind, format, mask, &event) {
        Ok(opened) => opened,
        Err(e) => {
            let _ = ready.send(Err(e));
            return delivery;
        }
    };
    // SAFETY: an initialised client.
    if let Err(e) = unsafe { client.Start() } {
        let _ = ready.send(Err(device_error("starting the capture stream", &e)));
        return delivery;
    }
    let _ = ready.send(Ok(()));
    let mut packets = ClientPackets(capture);
    let timeout_ms = if kind.event_driven() { 100 } else { 10 };
    let handles = [stop.handle(), event.handle()];
    loop {
        // SAFETY: two live event handles.
        let woke = unsafe { WaitForMultipleObjects(&handles, false, timeout_ms) };
        if woke == WAIT_OBJECT_0 {
            break; // stop
        }
        if !wake(&guard, &mut packets, &mut delivery) {
            break; // ended: counted in `ended_by` or `panics`
        }
    }
    // SAFETY: a started client, stopped once.
    let _ = unsafe { client.Stop() };
    drop(packets);
    drop(client);
    if let Some(handle) = mmcss {
        // SAFETY: the handle AvSetMmThreadCharacteristicsW returned on this thread.
        let _ = unsafe { AvRevertMmThreadCharacteristics(handle) };
    }
    drop(com);
    delivery
}

/// A started stream: its thread and the event that stops it.
pub(crate) struct Running {
    thread: JoinHandle<Delivery>,
    stop: Arc<Event>,
}

impl Running {
    /// Starts capture of `kind` in `format` into `sink`. It returns once the stream is up, or with
    /// the reason it would not come up (the sink is then dropped).
    pub(crate) fn start(
        kind: StreamKind,
        format: StreamFormat,
        mask: u32,
        sink: Box<dyn AudioSink>,
        guard: RealtimeGuard,
        clock: WinClock,
        counters: Arc<Counters>,
    ) -> Result<Self, PlatformError> {
        let stop = Arc::new(Event::new()?);
        let delivery = Delivery::new(
            sink,
            format,
            clock,
            kind.continuous(),
            Arc::clone(&counters),
        );
        let setup = ThreadSetup {
            kind,
            format,
            mask,
            delivery,
            guard,
            stop: Arc::clone(&stop),
            counters,
        };
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("ink-capture".into())
            .spawn(move || run(setup, ready_tx))
            .map_err(|e| PlatformError::Failed(format!("could not start a capture thread: {e}")))?;
        match ready.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => Ok(Self { thread, stop }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                // The thread is stuck bringing the stream up (an activation that never
                // completes). It is told to stop and left to finish on its own; it holds only its
                // own stream and the sink, which it drops when it ends.
                stop.set();
                Err(PlatformError::Device(format!(
                    "the capture stream did not start within {} s",
                    START_TIMEOUT.as_secs()
                )))
            }
        }
    }

    /// Stops the thread and drops the sink on this thread.
    pub(crate) fn stop(self) {
        self.stop.set();
        // A join error is a panic outside the guarded wake (a bug in setup or teardown); the sink
        // went with the thread.
        if let Ok(delivery) = self.thread.join() {
            drop(delivery.into_sink());
        }
    }
}

/// A finished session's [`SourceStats`], or an error when it was cut short.
pub(crate) fn session_result(stats: IoStats, what: &str) -> Result<SourceStats, PlatformError> {
    if stats.panics > 0 {
        return Err(PlatformError::Failed(format!(
            "{what}: the capture sink panicked on the capture thread; delivery stopped after {} \
             frames",
            stats.frames
        )));
    }
    if stats.ended_by != 0 {
        let hr = HRESULT(stats.ended_by);
        return Err(PlatformError::Device(format!(
            "{what}: the stream ended after {} frames ({}); the device was removed or changed \
             format; open the stream again",
            stats.frames,
            windows::core::Error::from_hresult(hr)
        )));
    }
    Ok(SourceStats {
        frames: stats.frames,
        discontinuities: stats.discontinuities + stats.skipped,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::VecDeque;
    use std::hint::black_box;

    use assert_no_alloc::{AllocDisabler, assert_no_alloc, violation_count};
    use ink_audio::capture_ring;
    use windows::Win32::Media::Audio::AUDCLNT_E_DEVICE_INVALIDATED;

    use super::*;

    // I4: this crate's unit tests run on the counting allocator (warn mode), so a test can prove
    // a wake allocation-free and the control test can prove the guard fires.
    #[global_allocator]
    static ALLOCATOR: AllocDisabler = AllocDisabler;

    /// A guard that runs each wake under `assert_no_alloc` and counts what it catches.
    pub(crate) fn counting_guard() -> (RealtimeGuard, Arc<AtomicU64>, Arc<AtomicU64>) {
        let violations = Arc::new(AtomicU64::new(0));
        let calls = Arc::new(AtomicU64::new(0));
        let (v, c) = (violations.clone(), calls.clone());
        let guard: RealtimeGuard = Arc::new(move |work: &mut dyn FnMut()| {
            let before = violation_count();
            assert_no_alloc(work);
            v.fetch_add(u64::from(violation_count() - before), Ordering::Relaxed);
            c.fetch_add(1, Ordering::Relaxed);
        });
        (guard, violations, calls)
    }

    const STEREO_48K: StreamFormat = StreamFormat {
        sample_rate: 48_000,
        channels: 2,
    };

    /// A scripted capture client: each queued packet is served once, then the queue says empty.
    /// Built before the wake, so serving it allocates nothing.
    struct Script {
        packets: VecDeque<(RawPacket, Vec<f32>)>,
        serving: Option<(RawPacket, Vec<f32>)>,
        /// Released buffers, kept so none is freed inside the guard. Preallocated.
        done: Vec<Vec<f32>>,
        fail_next_size: Option<HRESULT>,
        released: u64,
    }

    impl Script {
        fn new() -> Self {
            Self {
                packets: VecDeque::with_capacity(64),
                serving: None,
                done: Vec::with_capacity(256),
                fail_next_size: None,
                released: 0,
            }
        }

        fn queue(
            &mut self,
            samples: Vec<f32>,
            channels: usize,
            flags: u32,
            position: u64,
            qpc: u64,
        ) {
            let frames = (samples.len() / channels) as u32;
            let packet = RawPacket {
                data: samples.as_ptr().cast(),
                frames,
                flags,
                device_position: position,
                qpc_100ns: qpc,
            };
            self.packets.push_back((packet, samples));
        }
    }

    impl PacketSource for Script {
        fn next_size(&mut self) -> Result<u32, HRESULT> {
            if let Some(hr) = self.fail_next_size.take() {
                return Err(hr);
            }
            Ok(self.packets.front().map_or(0, |(p, _)| p.frames))
        }

        fn get(&mut self) -> Result<RawPacket, HRESULT> {
            let (packet, samples) = self.packets.pop_front().expect("next_size said one");
            self.serving = Some((packet, samples));
            Ok(packet)
        }

        fn release(&mut self, frames: u32) -> Result<(), HRESULT> {
            let (packet, samples) = self.serving.take().expect("a packet out");
            assert_eq!(packet.frames, frames);
            // Kept past the wake: dropping it here would free inside the guard. `done` has room,
            // so the push does not allocate either.
            assert!(self.done.len() < self.done.capacity(), "script too long");
            self.done.push(samples);
            self.released += 1;
            Ok(())
        }
    }

    fn delivery(sink: Box<dyn AudioSink>, continuous: bool) -> (Delivery, Arc<Counters>) {
        let counters = Arc::new(Counters::default());
        let clock = WinClock::new().unwrap();
        let d = Delivery::new(sink, STEREO_48K, clock, continuous, Arc::clone(&counters));
        (d, counters)
    }

    const SILENT: u32 = AUDCLNT_BUFFERFLAGS_SILENT.0 as u32;
    const GAP: u32 = AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32;
    const BAD_TIME: u32 = AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32;

    #[test]
    fn control_the_guard_catches_an_allocation() {
        let (guard, violations, calls) = counting_guard();
        guard(&mut || {
            black_box(Vec::<u8>::with_capacity(32));
        });
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            violations.load(Ordering::Relaxed),
            2,
            "an allocation and its free"
        );
    }

    /// I4: a wake draining real, silent and badly-stamped packets into `ink-audio`'s ring.
    #[test]
    fn a_wake_into_the_ring_allocates_nothing() {
        let (guard, violations, calls) = counting_guard();
        let (producer, mut consumer) = capture_ring(STEREO_48K, Duration::from_secs(2)).unwrap();
        let (mut delivery, counters) = delivery(Box::new(producer), true);
        let mut script = Script::new();
        for wake_index in 0..50u64 {
            let base = wake_index * 3 * 480;
            script.queue(vec![0.25; 960], 2, 0, base, 10_000 + base);
            script.queue(vec![0.0; 960], 2, SILENT, base + 480, 10_000 + base + 480);
            script.queue(vec![0.5; 960], 2, BAD_TIME, base + 960, 0);
            assert!(wake(&guard, &mut script, &mut delivery));
            let mut blocks = 0;
            while consumer.pop().is_some() {
                blocks += 1;
            }
            assert_eq!(blocks, 3, "wake {wake_index}");
        }
        assert_eq!(
            calls.load(Ordering::Relaxed),
            50,
            "the guard wrapped every wake"
        );
        assert_eq!(violations.load(Ordering::Relaxed), 0, "and none allocated");
        let stats = counters.snapshot();
        assert_eq!(stats.callbacks, 50);
        assert_eq!(stats.packets, 150);
        assert_eq!(stats.silent_packets, 50);
        assert_eq!(stats.frames, 150 * 480);
        assert_eq!(stats.untimed, 50);
        assert_eq!(stats.discontinuities, 0);
        assert_eq!(script.released, 150);
    }

    /// I4 control: the same path with a sink that allocates is caught.
    #[test]
    fn control_an_allocating_sink_is_caught_through_the_wake() {
        struct Allocates;
        impl AudioSink for Allocates {
            fn push(&mut self, block: &AudioBlock<'_>) {
                black_box(block.samples.to_vec());
            }
        }
        let (guard, violations, _) = counting_guard();
        let (mut delivery, _) = delivery(Box::new(Allocates), true);
        let mut script = Script::new();
        script.queue(vec![0.1; 96], 2, 0, 0, 1);
        assert!(wake(&guard, &mut script, &mut delivery));
        assert!(violations.load(Ordering::Relaxed) > 0);
    }

    /// Records what reaches the sink (outside any guard).
    #[derive(Clone, Default)]
    struct Recorder(Arc<std::sync::Mutex<Vec<(usize, bool, u64)>>>);

    impl AudioSink for Recorder {
        fn push(&mut self, block: &AudioBlock<'_>) {
            let zeros = block.samples.iter().all(|&s| s == 0.0);
            self.0
                .lock()
                .unwrap()
                .push((block.frames(), zeros, block.host_time_ns));
        }
    }

    fn unguarded() -> RealtimeGuard {
        ink_audio::unguarded()
    }

    #[test]
    fn a_silent_packet_is_zeros_whatever_its_buffer_holds_and_long_ones_split() {
        let recorder = Recorder::default();
        let (mut delivery, _) = delivery(Box::new(recorder.clone()), false);
        let mut script = Script::new();
        // 2500 frames flagged silent over a buffer of garbage.
        script.queue(vec![0.9; 5_000], 2, SILENT, 0, 1_000);
        assert!(wake(&unguarded(), &mut script, &mut delivery));
        let blocks = recorder.0.lock().unwrap().clone();
        assert_eq!(
            blocks.iter().map(|b| b.0).collect::<Vec<_>>(),
            [1_024, 1_024, 452]
        );
        assert!(blocks.iter().all(|b| b.1), "all zeros");
        // Stamps advance by the frames before each block: 1024 frames at 48 kHz.
        assert_eq!(blocks[0].2, 100_000);
        assert_eq!(blocks[1].2, 100_000 + 1_024 * 1_000_000_000 / 48_000);
    }

    #[test]
    fn host_time_comes_from_the_qpc_position() {
        let recorder = Recorder::default();
        let (mut delivery, _) = delivery(Box::new(recorder.clone()), false);
        let mut script = Script::new();
        script.queue(vec![0.1; 20], 2, 0, 0, 123_456);
        assert!(wake(&unguarded(), &mut script, &mut delivery));
        assert_eq!(recorder.0.lock().unwrap()[0].2, 12_345_600);
    }

    #[test]
    fn the_first_packets_discontinuity_flag_is_not_a_gap_but_later_ones_are() {
        let (mut delivery, counters) = delivery(Box::new(Recorder::default()), false);
        let mut script = Script::new();
        script.queue(vec![0.1; 20], 2, GAP, 0, 1);
        script.queue(vec![0.1; 20], 2, 0, 10, 2);
        script.queue(vec![0.1; 20], 2, GAP, 20, 3);
        assert!(wake(&unguarded(), &mut script, &mut delivery));
        assert_eq!(counters.snapshot().discontinuities, 1);
    }

    #[test]
    fn a_position_jump_is_a_gap_on_the_mic_but_idle_on_loopback() {
        for (continuous, gaps) in [(true, 1), (false, 0)] {
            let (mut delivery, counters) = delivery(Box::new(Recorder::default()), continuous);
            let mut script = Script::new();
            script.queue(vec![0.1; 20], 2, 0, 0, 1);
            script.queue(vec![0.1; 20], 2, 0, 5_000, 2);
            assert!(wake(&unguarded(), &mut script, &mut delivery));
            assert_eq!(
                counters.snapshot().discontinuities,
                gaps,
                "continuous {continuous}"
            );
        }
    }

    #[test]
    fn a_null_buffer_is_skipped_not_read() {
        let recorder = Recorder::default();
        let (mut delivery, counters) = delivery(Box::new(recorder.clone()), true);
        let mut script = Script::new();
        script.queue(vec![0.1; 20], 2, 0, 0, 1);
        script.packets[0].0.data = std::ptr::null();
        assert!(wake(&unguarded(), &mut script, &mut delivery));
        assert!(recorder.0.lock().unwrap().is_empty());
        let stats = counters.snapshot();
        assert_eq!(stats.skipped, 1);
        assert_eq!(stats.frames, 0);
        assert_eq!(
            session_result(stats, "mic").unwrap().discontinuities,
            1,
            "a skipped packet is a gap"
        );
    }

    #[test]
    fn a_client_error_ends_the_stream_and_is_reported_at_stop() {
        let (mut delivery, counters) = delivery(Box::new(Recorder::default()), true);
        let mut script = Script::new();
        script.fail_next_size = Some(AUDCLNT_E_DEVICE_INVALIDATED);
        assert!(!wake(&unguarded(), &mut script, &mut delivery));
        let stats = counters.snapshot();
        assert_eq!(stats.ended_by, AUDCLNT_E_DEVICE_INVALIDATED.0);
        let error = session_result(stats, "the microphone").unwrap_err();
        assert!(
            matches!(&error, PlatformError::Device(m) if m.contains("removed or changed format")),
            "{error}"
        );
    }

    #[test]
    fn a_panicking_sink_ends_the_stream_and_is_reported() {
        struct Panics;
        impl AudioSink for Panics {
            fn push(&mut self, _: &AudioBlock<'_>) {
                panic!("sink bug");
            }
        }
        let (mut delivery, counters) = delivery(Box::new(Panics), true);
        let mut script = Script::new();
        script.queue(vec![0.1; 20], 2, 0, 0, 1);
        assert!(!wake(&unguarded(), &mut script, &mut delivery));
        let stats = counters.snapshot();
        assert_eq!(stats.panics, 1);
        assert!(session_result(stats, "mic").is_err());
    }

    #[test]
    fn a_multichannel_device_keeps_its_layout_or_falls_back_to_stereo() {
        let surround = StreamFormat {
            sample_rate: 48_000,
            channels: 6,
        };
        let mix = (surround, 0x3F);
        // Taken as it is.
        assert_eq!(choose_layout(mix, |_, _| true), Some(mix));
        // Refused in 5.1: stereo at the same rate, with the stereo mask.
        let mut tried = Vec::new();
        let chosen = choose_layout(mix, |format, mask| {
            tried.push((format.channels, mask));
            format.channels == 2
        });
        assert_eq!(
            chosen,
            Some((
                StreamFormat {
                    sample_rate: 48_000,
                    channels: 2
                },
                0x3
            ))
        );
        assert_eq!(tried, [(6, 0x3F), (2, 0x3)]);
        // Refused in both: no layout.
        assert_eq!(choose_layout(mix, |_, _| false), None);
        // Mono and stereo are never tried: they are what every shared-mode stream takes.
        let stereo = (
            StreamFormat {
                sample_rate: 44_100,
                channels: 2,
            },
            0x3,
        );
        assert_eq!(
            choose_layout(stereo, |_, _| panic!("not tried")),
            Some(stereo)
        );
    }

    #[test]
    fn a_float_format_carries_the_devices_mask() {
        let wave = float_format(
            StreamFormat {
                sample_rate: 48_000,
                channels: 6,
            },
            0x3F,
        );
        assert_eq!({ wave.dwChannelMask }, 0x3F);
        assert_eq!({ wave.Format.nBlockAlign }, 24);
    }

    #[test]
    fn a_float_format_describes_itself_fully() {
        let wave = float_format(
            StreamFormat {
                sample_rate: 32_000,
                channels: 1,
            },
            default_mask(1),
        );
        let header = wave.Format;
        assert_eq!({ header.wFormatTag }, WAVE_FORMAT_EXTENSIBLE as u16);
        assert_eq!({ header.nSamplesPerSec }, 32_000);
        assert_eq!({ header.nBlockAlign }, 4);
        assert_eq!({ header.nAvgBytesPerSec }, 128_000);
        assert_eq!({ header.cbSize }, 22);
        assert_eq!({ wave.dwChannelMask }, 0x4);
        assert_eq!({ wave.SubFormat }, KSDATAFORMAT_SUBTYPE_IEEE_FLOAT);
    }
}
