//! FileReplaySource: the replay harness, driven through the same sink contract as a device, into
//! the real ring and chunk store.

mod common;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{MONO_16K, STEREO_48K, TempDir, at, signal, snapshot, write_wav_f32, write_wav_i16};
use ink_audio::{
    ChunkStore, FileReplaySource, Overruns, Pacing, RateVerdict, WriterSummary, capture_ring,
};
use ink_core::mock::MockClock;
use ink_core::{AudioBlock, AudioSink, AudioSource, Channel, Clock, SourceStats, StreamFormat};

const T0: u64 = 42_000_000_000;

fn clock_at(ns: u64) -> Arc<dyn Clock> {
    Arc::new(MockClock::new(ns, 1_700_000_000_000))
}

/// Replays `source` through a ring into `store` with a pump thread draining concurrently, the way
/// the meeting chain will. The ring is sized to hold the whole file, so no schedule can overrun it.
fn replay_into(mut source: FileReplaySource, store: &ChunkStore) -> (WriterSummary, Overruns) {
    let format = source.format();
    let seconds = source.total_frames() / u64::from(format.sample_rate) + 1;
    let (producer, mut consumer) = capture_ring(format, Duration::from_secs(seconds)).unwrap();
    let mut writer = store.writer(source.channel(), format).unwrap();
    let pump = std::thread::spawn(move || {
        loop {
            let gone = consumer.is_abandoned();
            match consumer.pop() {
                Some(c) => writer.write(&c.block, c.dropped_frames_before).unwrap(),
                None if gone => break,
                None => std::thread::yield_now(),
            }
        }
        (writer.finish().unwrap(), consumer.overruns())
    });
    source.start(Box::new(producer)).unwrap();
    let stats = source.wait().unwrap();
    assert_eq!(stats.frames, source.total_frames());
    pump.join().unwrap()
}

#[test]
fn replaying_a_fixture_twice_gives_byte_identical_chunks() {
    let tmp = TempDir::new("determinism");
    let wav = tmp.join("fixture.wav");
    let decoded = write_wav_i16(&wav, MONO_16K, &signal(56_000, MONO_16K, 1));

    let mut runs = Vec::new();
    for (run, start) in [("a", T0), ("b", T0), ("c", T0 + 1)] {
        let dir = tmp.join(run);
        let store = ChunkStore::open(&dir)
            .unwrap()
            .with_chunk_duration(Duration::from_secs(1));
        let source = FileReplaySource::open(&wav, Channel::Mic, clock_at(start)).unwrap();
        let (summary, overruns) = replay_into(source, &store);
        assert_eq!(overruns, Overruns::default());
        assert_eq!(
            (summary.chunks, summary.frames, summary.gaps),
            (4, 56_000, 0)
        );
        let samples: Vec<f32> = store
            .chunks(Channel::Mic)
            .unwrap()
            .iter()
            .flat_map(|c| store.read(c).unwrap())
            .collect();
        assert_eq!(
            samples, decoded,
            "run {run}: the fixture, sample for sample"
        );
        runs.push(snapshot(&dir));
    }
    assert_eq!(runs[0].len(), 4);
    assert_eq!(
        runs[0], runs[1],
        "same fixture, same start: byte-identical chunks"
    );
    // The host times come from the clock the test controls: move it and the headers move with it.
    assert_ne!(runs[0], runs[2]);
    let names = |r: &Vec<(String, Vec<u8>)>| r.iter().map(|f| f.0.clone()).collect::<Vec<_>>();
    assert_eq!(names(&runs[0]), names(&runs[2]));
}

/// One push as recorded: (host time, frames, format, thread).
type Push = (u64, usize, StreamFormat, std::thread::ThreadId);

/// Records every push.
#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<Push>>>);

impl AudioSink for Recorder {
    fn push(&mut self, block: &AudioBlock<'_>) {
        self.0.lock().unwrap().push((
            block.host_time_ns,
            block.frames(),
            block.format,
            std::thread::current().id(),
        ));
    }
}

#[test]
fn replay_delivers_fixed_blocks_with_synthetic_monotonic_host_times_from_its_own_thread() {
    let audio = signal(1_000, MONO_16K, 2);
    let mut source =
        FileReplaySource::from_samples(audio, MONO_16K, Channel::Far, clock_at(T0)).unwrap();
    assert_eq!(source.format(), MONO_16K);
    assert_eq!(source.channel(), Channel::Far);
    let recorder = Recorder::default();
    source.start(Box::new(recorder.clone())).unwrap();
    let stats = source.wait().unwrap();
    assert_eq!(
        stats,
        SourceStats {
            frames: 1_000,
            discontinuities: 0
        }
    );

    let pushes = recorder.0.lock().unwrap().clone();
    let shape: Vec<(u64, usize)> = pushes.iter().map(|p| (p.0, p.1)).collect();
    // 20 ms blocks (320 frames at 16 kHz); the tail is short, never padded.
    assert_eq!(
        shape,
        vec![
            (T0, 320),
            (at(T0, 320, 16_000), 320),
            (at(T0, 640, 16_000), 320),
            (at(T0, 960, 16_000), 40)
        ]
    );
    assert!(pushes.iter().all(|p| p.2 == MONO_16K));
    let replay_thread = pushes[0].3;
    assert_ne!(replay_thread, std::thread::current().id());
    assert!(pushes.iter().all(|p| p.3 == replay_thread));
}

#[test]
fn replay_reads_16_bit_and_float_wav_files() {
    let tmp = TempDir::new("formats");
    let pcm16 = tmp.join("mono16.wav");
    let decoded = write_wav_i16(&pcm16, MONO_16K, &signal(800, MONO_16K, 3));
    let float = tmp.join("stereo-float.wav");
    let stereo = signal(960, STEREO_48K, 4);
    write_wav_f32(&float, STEREO_48K, &stereo);

    for (path, format, expected) in [(pcm16, MONO_16K, decoded), (float, STEREO_48K, stereo)] {
        let mut source = FileReplaySource::open(&path, Channel::Mic, clock_at(0))
            .unwrap()
            .with_block_frames(256);
        assert_eq!(source.format(), format);
        let got = Arc::new(Mutex::new(Vec::new()));
        struct Collect(Arc<Mutex<Vec<f32>>>);
        impl AudioSink for Collect {
            fn push(&mut self, block: &AudioBlock<'_>) {
                self.0.lock().unwrap().extend_from_slice(block.samples);
            }
        }
        source.start(Box::new(Collect(got.clone()))).unwrap();
        source.wait().unwrap();
        assert_eq!(*got.lock().unwrap(), expected);
    }
}

#[test]
fn opening_a_missing_or_invalid_file_is_an_error() {
    let tmp = TempDir::new("missing");
    assert!(FileReplaySource::open(tmp.join("nope.wav"), Channel::Mic, clock_at(0)).is_err());
    let junk = tmp.join("junk.wav");
    std::fs::write(&junk, b"not a wav file").unwrap();
    assert!(FileReplaySource::open(&junk, Channel::Mic, clock_at(0)).is_err());
    let odd =
        FileReplaySource::from_samples(vec![0.0f32; 3], STEREO_48K, Channel::Mic, clock_at(0));
    assert!(odd.is_err(), "samples must be whole frames");
}

/// A sink that takes a while per push and reports whether a push is running, how many finished,
/// and whether it has been dropped.
#[derive(Clone, Default)]
struct SlowSink {
    in_push: Arc<AtomicBool>,
    pushes: Arc<AtomicU64>,
    dropped: Arc<AtomicBool>,
}

impl AudioSink for SlowSink {
    fn push(&mut self, _block: &AudioBlock<'_>) {
        self.in_push.store(true, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(30));
        self.pushes.fetch_add(1, Ordering::SeqCst);
        self.in_push.store(false, Ordering::SeqCst);
    }
}

impl Drop for SlowSink {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[test]
fn stop_does_not_return_while_a_push_is_running_and_drops_the_sink() {
    let audio = signal(16_000, MONO_16K, 5);
    let mut source =
        FileReplaySource::from_samples(audio, MONO_16K, Channel::Mic, clock_at(T0)).unwrap();
    let sink = SlowSink::default();
    let probe = SlowSink {
        in_push: sink.in_push.clone(),
        pushes: sink.pushes.clone(),
        dropped: Arc::new(AtomicBool::new(false)),
    };
    let dropped = sink.dropped.clone();
    source.start(Box::new(sink)).unwrap();

    // Stop while a push is (very likely) in progress.
    while !probe.in_push.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }
    let stats = source.stop().unwrap();
    assert!(
        !probe.in_push.load(Ordering::SeqCst),
        "no push running after stop"
    );
    assert!(
        dropped.load(Ordering::SeqCst),
        "the sink was dropped before stop returned"
    );
    let done = probe.pushes.load(Ordering::SeqCst);
    assert!(
        (1..50).contains(&done),
        "stopped early, after {done} pushes"
    );
    assert_eq!(stats.frames, done * 320);
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(probe.pushes.load(Ordering::SeqCst), done, "and none after");
}

#[test]
fn start_twice_is_refused_and_stop_twice_gives_zeroed_stats() {
    let audio = signal(640, MONO_16K, 6);
    let mut source =
        FileReplaySource::from_samples(audio, MONO_16K, Channel::Mic, clock_at(T0)).unwrap();
    assert_eq!(
        source.stop().unwrap(),
        SourceStats::default(),
        "never started"
    );
    source.start(Box::new(Recorder::default())).unwrap();
    assert!(source.start(Box::new(Recorder::default())).is_err());
    assert_eq!(source.wait().unwrap().frames, 640);
    assert_eq!(source.stop().unwrap(), SourceStats::default());
    assert_eq!(source.wait().unwrap(), SourceStats::default());

    // A stopped source can start again, from the beginning, at the clock's time then.
    let recorder = Recorder::default();
    source.start(Box::new(recorder.clone())).unwrap();
    assert_eq!(source.wait().unwrap().frames, 640);
    assert_eq!(recorder.0.lock().unwrap()[0].0, T0);
}

#[test]
fn a_sink_that_panics_is_reported_by_stop() {
    struct Panics;
    impl AudioSink for Panics {
        fn push(&mut self, _block: &AudioBlock<'_>) {
            panic!("sink failure (expected by this test)");
        }
    }
    let mut source = FileReplaySource::from_samples(
        signal(640, MONO_16K, 7),
        MONO_16K,
        Channel::Mic,
        clock_at(0),
    )
    .unwrap();
    source.start(Box::new(Panics)).unwrap();
    assert!(source.wait().is_err());
}

#[test]
fn a_mislabelled_rate_is_caught_end_to_end_and_the_audio_kept() {
    // Replay 48 kHz audio from a "device" that declares 16 kHz: the incident, from source to disk.
    let tmp = TempDir::new("mislabelled");
    let audio = signal(
        96_000,
        StreamFormat {
            sample_rate: 48_000,
            channels: 1,
        },
        8,
    );
    let source = FileReplaySource::from_samples(
        audio.clone(),
        StreamFormat {
            sample_rate: 48_000,
            channels: 1,
        },
        Channel::Mic,
        clock_at(T0),
    )
    .unwrap()
    .declaring_rate(16_000);
    assert_eq!(source.format(), MONO_16K);

    let store = ChunkStore::open(tmp.path()).unwrap();
    let (summary, _) = replay_into(source, &store);
    assert_eq!(
        summary.rate,
        RateVerdict::Mismatch {
            declared_hz: 16_000,
            measured_hz: 48_000
        }
    );
    let chunks = store.chunks(Channel::Mic).unwrap();
    assert!(chunks.iter().all(|c| c.format == MONO_16K));
    let kept: Vec<f32> = chunks.iter().flat_map(|c| store.read(c).unwrap()).collect();
    assert_eq!(kept, audio, "not resampled");
}

#[test]
fn real_time_pacing_takes_as_long_as_the_audio() {
    let audio = signal(3_200, MONO_16K, 9); // 200 ms
    let mut source = FileReplaySource::from_samples(audio, MONO_16K, Channel::Mic, clock_at(0))
        .unwrap()
        .with_pacing(Pacing::RealTime);
    let started = Instant::now();
    source.start(Box::new(Recorder::default())).unwrap();
    source.wait().unwrap();
    // The last block goes out at 180 ms; a lower bound only, so a slow machine cannot fail it.
    assert!(
        started.elapsed() >= Duration::from_millis(175),
        "{:?}",
        started.elapsed()
    );
}
