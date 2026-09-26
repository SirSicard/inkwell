//! The step's two-channel fixture: AMI meeting IS1009a, 30 s, the mic from one headset and the far
//! end mixed from the other three (`fixtures/ami/IS1009a.json` says exactly how). Every remote
//! headset speaks in it, so the far end has three speakers for rule 5.
//!
//! Replayed through `FileReplaySource` (architecture rule 7) into capture rings, the pump, the live
//! chain and the final pass. CI has no models, so the VAD is scripted (an energy threshold) and
//! the engines and diarizer are mocks. A real VAD is tested on this audio, never on synthetic
//! speech: the Silero binding scores every synthetic speech fixture as "no speech". That test
//! needs the Silero binding (not in this crate's tree yet); it belongs here, `#[ignore]`d.
//!
//! **What the diarization assertions prove, and what they do not.** The diarizer here is
//! `MockDiarizer`: it ignores the audio and returns the same scripted turns every time. So these
//! tests prove the plumbing around diarization: only the far end reaches the diarizer, rule 5 is
//! applied to its turns, far regions are cut where the turns change speaker, and each segment gets
//! the speaker holding most of it. They prove nothing about whether a diarizer finds the three
//! people who actually speak in this excerpt: that takes the real diarizer, in an ignored test.

mod meeting_rig;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ink_audio::gain::{TARGET_PEAK, robust_peak, to_dbfs};
use ink_audio::{ChunkStore, FileReplaySource, capture_ring};
use ink_core::mock::{MemStore, MockClock};
use ink_core::{AudioSource, CancelToken, Channel, EventSink, Segment, Store, StreamFormat};
use ink_pipeline::capture::SideCapture;
use ink_pipeline::meeting::events::MeetingEvent;
use ink_pipeline::meeting::{MeetingChain, MeetingServices, MeetingStart};
use meeting_rig::*;
use sha2::{Digest, Sha256};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/ami")
}

fn manifest() -> serde_json::Value {
    let text = std::fs::read_to_string(fixtures().join("IS1009a.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn sha256(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap();
    Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The committed files are the ones the manifest describes, byte for byte.
#[test]
fn the_fixture_matches_its_manifest() {
    let m = manifest();
    assert_eq!(m["meeting"], "IS1009a");
    assert_eq!(m["licence"], "CC-BY-4.0");
    let (start, end) = (
        m["range"]["start_sample"].as_u64().unwrap(),
        m["range"]["end_sample"].as_u64().unwrap(),
    );
    assert_eq!(end - start, 30 * 16_000);
    for channel in m["channels"].as_array().unwrap() {
        let path = fixtures().join(channel["file"].as_str().unwrap());
        assert_eq!(sha256(&path), channel["sha256"].as_str().unwrap());
        let reader = hound::WavReader::open(&path).unwrap();
        let spec = reader.spec();
        assert_eq!(
            (spec.channels, spec.sample_rate, spec.bits_per_sample),
            (1, 16_000, 16)
        );
        assert_eq!(u64::from(reader.duration()), end - start);
    }
}

/// Carry-over: the corpus files are not in the repository, so this regenerates the excerpt from
/// `$INK_BENCH_DIR` with the manifest's recipe, in Rust, and compares it with the committed files
/// sample for sample. It checks the manifest describes the files (the Python script checks the
/// same from the other side).
#[test]
#[ignore = "reads the AMI corpus from $INK_BENCH_DIR"]
fn the_fixture_regenerates_from_the_corpus() {
    let bench = std::env::var("INK_BENCH_DIR").expect("set INK_BENCH_DIR");
    let m = manifest();
    let source = Path::new(&bench).join(m["source"]["dir"].as_str().unwrap());
    let (start, end) = (
        m["range"]["start_sample"].as_u64().unwrap() as u32,
        m["range"]["end_sample"].as_u64().unwrap() as u32,
    );
    let read = |path: &Path, from: u32, to: u32| -> Vec<i32> {
        let mut r = hound::WavReader::open(path).unwrap();
        r.seek(from).unwrap();
        r.samples::<i16>()
            .take((to - from) as usize)
            .map(|s| i32::from(s.unwrap()))
            .collect()
    };
    for (name, digest) in m["source"]["sha256"].as_object().unwrap() {
        assert_eq!(&sha256(&source.join(name)), digest, "{name}");
    }
    for channel in m["channels"].as_array().unwrap() {
        let gain = channel["gain"].as_f64().unwrap();
        let sources: Vec<Vec<i32>> = channel["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| read(&source.join(s.as_str().unwrap()), start, end))
            .collect();
        let made: Vec<i32> = (0..sources[0].len())
            .map(|i| {
                let sum: i32 = sources.iter().map(|s| s[i]).sum();
                ((f64::from(sum) * gain).round_ties_even() as i32).clamp(-32_768, 32_767)
            })
            .collect();
        let committed = read(
            &fixtures().join(channel["file"].as_str().unwrap()),
            0,
            end - start,
        );
        assert_eq!(made.len(), committed.len());
        let differ = made.iter().zip(&committed).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "{}: {differ} samples differ", channel["file"]);
    }
}

fn assert_monotonic(segments: &[Segment]) {
    for channel in [Channel::Mic, Channel::Far] {
        let mut last = 0;
        for s in segments.iter().filter(|s| s.channel == channel) {
            assert!(
                s.end_ms >= s.start_ms && s.start_ms >= last,
                "{segments:#?}"
            );
            last = s.start_ms;
        }
    }
    assert!(segments.windows(2).all(|w| w[0].start_ms <= w[1].start_ms));
}

/// The step's check: replayed through the whole chain, the fixture gives revision 2, every
/// segment on the side it was captured on, and times in order. The far end goes to the diarizer
/// (a mock with scripted turns: see the module docs) and its labels come through rule 5.
#[test]
fn the_ami_fixture_gives_revision_2_correct_you_and_them_and_monotonic_times() {
    let clock = Arc::new(MockClock::new(T0_NS, T0_UNIX_MS));
    let dir = TempDir::new("ami");
    let chunks = ChunkStore::open(dir.path().join("record")).unwrap();
    let store = Arc::new(MemStore::new());
    let engine = Final::new(numbered());
    let live = Onsets::new();
    // Scripted turns, not a diarization of this audio: three speakers over the far end's speech,
    // end to end.
    let diarizer = diarizer(&[
        ("spk0", 0, 5_000),
        ("spk1", 5_000, 10_000),
        ("spk2", 10_000, 30_000),
    ]);
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink_events = events.clone();
    let sink: EventSink<MeetingEvent> = Arc::new(move |e| sink_events.lock().unwrap().push(e));
    let mut chain = MeetingChain::start(
        MeetingServices {
            live: Some(live.clone()),
            offline: engine.clone(),
            diarizer: Some(diarizer.clone()),
            store: store.clone(),
            clock: clock.clone(),
            llm: None,
        },
        Default::default(),
        // Speech above −35 dBFS: the headsets' room tone and crosstalk sit at −40 to −45.
        vad_source(|_| Box::new(EnergyVad(-35.0))),
        sink,
        MeetingStart::default(),
    )
    .unwrap();
    let record = chain.record().clone();

    // Capture: each side replayed into its own ring, then drained by the pump.
    let mut sides = Vec::new();
    for (channel, file) in [
        (Channel::Mic, "IS1009a-mic.wav"),
        (Channel::Far, "IS1009a-far.wav"),
    ] {
        let (tx, rx) = capture_ring(StreamFormat::CANONICAL, Duration::from_secs(31)).unwrap();
        let mut source =
            FileReplaySource::open(fixtures().join(file), channel, clock.clone()).unwrap();
        source.start(Box::new(tx)).unwrap();
        let stats = source.wait().unwrap();
        assert_eq!(stats.frames, 30 * 16_000);
        sides.push(SideCapture::new(channel, rx, chunks.clone()));
    }
    for side in sides {
        let summary = side.finish(
            &mut |b| chain.push_audio(b.channel, &b.samples, b.host_time_ns, b.dropped_frames),
            &mut |i| panic!("capture: {i}"),
        );
        assert_eq!((summary.chunks, summary.captured_ms), (3, 30_000));
        chain.capture_ended(summary);
    }
    let ended = chain.stop();
    let live_segments = store.segments(&record).unwrap();
    assert!(live_segments.iter().any(|s| s.channel == Channel::Mic));
    assert!(live_segments.iter().any(|s| s.channel == Channel::Far));

    let outcome = ended.finalize(&chunks, &CancelToken::new()).unwrap();
    assert!(outcome.superseded, "{:?}", events.lock().unwrap());
    assert_eq!(outcome.revision, Some(2));
    assert_eq!(store.record(&record).unwrap().unwrap().revision, 2);
    let finals = store.segments(&record).unwrap();
    for s in &finals {
        let side = match s.channel {
            Channel::Mic => "mic words",
            Channel::Far => "far words",
        };
        assert!(s.text.starts_with(side), "{s:?}");
    }
    assert_monotonic(&finals);
    // You never reached the diarizer; the far end did, and its scripted speakers came through.
    assert!(
        finals
            .iter()
            .filter(|s| s.channel == Channel::Mic)
            .all(|s| s.speaker.is_none())
    );
    let mut speakers: Vec<_> = finals.iter().filter_map(|s| s.speaker.clone()).collect();
    speakers.sort();
    speakers.dedup();
    assert!(speakers.len() >= 2, "{finals:#?}");
    assert_eq!(diarizer.calls(), 1);
    let d = outcome.diarization.expect("diarized");
    assert!(d.labelled && d.substantial >= 2, "{d:?}");
    // Speech regions only: neither side sent all 30 s.
    for pass in [outcome.mic, outcome.far] {
        assert_eq!(pass.chunks_written, Some(3));
        assert_eq!((pass.chunks, pass.captured_ms), (3, 30_000));
        assert!(pass.regions >= 1, "{pass:?}");
        assert!(pass.speech_ms < 30_000, "{pass:?}");
        assert_eq!(pass.failed_regions, 0);
    }
    // Levels on real audio. One gain per window, learned from its speech: the loudest speech
    // lands near the target and quieter remarks keep their level relative to it (as a slow AGC
    // would); healthy audio is never attenuated. Measured here: calls from 18 dB under the target
    // to 3 dB over, every one far above the −65 dBFS where engines return nothing.
    let inputs = engine.inputs.lock().unwrap();
    for channel in [Channel::Mic, Channel::Far] {
        let offs: Vec<f32> = inputs
            .iter()
            .filter(|(c, _)| *c == channel)
            .map(|(_, a)| to_dbfs(robust_peak(a)) - to_dbfs(TARGET_PEAK))
            .collect();
        let loudest = offs.iter().copied().fold(f32::MIN, f32::max);
        assert!((-3.0..=6.0).contains(&loudest), "{channel:?}: {offs:?}");
    }
    for (channel, audio) in inputs.iter() {
        assert!(
            rms_dbfs(audio) > -40.0,
            "{channel:?}: {:.1} dBFS",
            rms_dbfs(audio)
        );
    }
    eprintln!(
        "AMI fixture: mic {:?}; far {:?}; diarization {:?}; {} final segments",
        outcome.mic,
        outcome.far,
        d,
        finals.len()
    );
}
