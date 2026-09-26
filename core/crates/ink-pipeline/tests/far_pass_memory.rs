//! Item 7 of the review: the far pass holds a bounded amount of audio however long the far end
//! is. The earlier path kept every far region for the whole pass and then copied them all into one
//! buffer for the diarizer: an hour of far speech was about 230 MB, briefly twice.
//!
//! Measured, not assumed: this binary counts every heap byte (a counting global allocator), and
//! the final pass of a 4-minute and a 12-minute far end must peak at the same few seconds' worth
//! of audio. It is its own test binary so no other test's allocations are counted.

mod meeting_rig;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ink_audio::ChunkStore;
use ink_core::mock::{MemStore, MockClock};
use ink_core::{
    AudioBlock, CancelToken, Channel, EngineError, EngineInfo, OfflineEngine, StreamFormat,
    TranscribeOptions, Transcript,
};
use ink_pipeline::meeting::{MeetingChain, MeetingServices, MeetingStart};
use meeting_rig::*;

struct Counting;

static NOW: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call is forwarded to the system allocator unchanged; the counters only observe.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract for `alloc` is passed on as is.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let now = NOW.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: as above.
        unsafe { System.dealloc(ptr, layout) };
        NOW.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: as above.
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            let now = if new_size >= layout.size() {
                NOW.fetch_add(new_size - layout.size(), Ordering::Relaxed) + new_size
                    - layout.size()
            } else {
                NOW.fetch_sub(layout.size() - new_size, Ordering::Relaxed)
                    - (layout.size() - new_size)
            };
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// An engine that keeps nothing of what it hears.
struct Forgetful;

impl OfflineEngine for Forgetful {
    fn info(&self) -> EngineInfo {
        EngineInfo {
            id: "forgetful".into(),
            jobs: vec![],
            licence: "MIT".into(),
        }
    }
    fn transcribe(&self, audio: &[f32], _: &TranscribeOptions) -> Result<Transcript, EngineError> {
        Ok(words("far words here", audio.len()))
    }
}

/// Bytes the final pass of a meeting with `minutes` of far-end speech held at its peak, above
/// what was held before it began.
fn peak_of_far_pass(minutes: usize) -> usize {
    let dir = TempDir::new("far-memory");
    let chunks = ChunkStore::open(dir.path().join("record")).unwrap();
    // Ten seconds of speech with its pauses, recorded again and again.
    let block = join(&[speech(9.0, -30.0, 131), silence(1.0)]);
    let mut writer = chunks
        .writer(Channel::Far, StreamFormat::CANONICAL)
        .unwrap();
    for k in 0..minutes * 6 {
        for (j, piece) in block.chunks(1_600).enumerate() {
            let at = (k * block.len() + j * 1_600) as u64 * 62_500;
            let b = AudioBlock {
                samples: piece,
                format: StreamFormat::CANONICAL,
                host_time_ns: T0_NS + at,
            };
            writer.write(&b, 0).unwrap();
        }
    }
    writer.finish().unwrap();
    drop(block);

    let diarizer = diarizer(&[("spk0", 0, 30_000), ("spk1", 30_000, 60_000)]);
    let chain = MeetingChain::start(
        MeetingServices {
            live: None,
            offline: Arc::new(Forgetful),
            diarizer: Some(diarizer.clone()),
            store: Arc::new(MemStore::new()),
            clock: Arc::new(MockClock::new(T0_NS, T0_UNIX_MS)),
            llm: None,
        },
        Default::default(),
        energy_vad(),
        Arc::new(|_| {}),
        MeetingStart::default(),
    )
    .unwrap();
    let ended = chain.stop();

    let before = NOW.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let outcome = ended.finalize(&chunks, &CancelToken::new()).unwrap();
    let peak = PEAK.load(Ordering::Relaxed) - before;
    assert_eq!(outcome.far.captured_ms, minutes as u64 * 60_000);
    assert!(outcome.far.regions > minutes, "{:?}", outcome.far);
    assert_eq!(
        diarizer.samples() / 16_000 / 60,
        (outcome.far.speech_ms / 60_000)
    );
    assert!(diarizer.largest_window() <= ink_core::MAX_DIARIZE_WINDOW);
    peak
}

#[test]
fn the_far_pass_holds_a_bounded_amount_of_audio_however_long_the_far_end() {
    let short = peak_of_far_pass(4);
    let long = peak_of_far_pass(12);
    // In seconds of 16 kHz f32 audio, for the message.
    let secs = |bytes: usize| bytes as f64 / 64_000.0;
    println!(
        "final pass peak: {:.1} MB ({:.0} s of audio) for 4 min, {:.1} MB ({:.0} s) for 12 min",
        short as f64 / 1e6,
        secs(short),
        long as f64 / 1e6,
        secs(long)
    );
    // Twelve minutes of far speech is about 11 minutes of regions, 42 MB once and 84 MB with the
    // copy the one-buffer path made. The streamed pass holds a few windows' worth, the same for
    // both lengths.
    assert!(
        long < short + short / 10,
        "the peak grew with the meeting: {short} bytes for 4 min, {long} for 12"
    );
    assert!(
        secs(long) < 360.0,
        "{:.0} s of audio held at once",
        secs(long)
    );
}
