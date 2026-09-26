//! The meeting chain on the real engines, on the committed AMI fixture (IS1009a, 30 s): Silero VAD
//! (on tract) in the live AGC and the final pass, Nemotron-3-Diarization (NeMo-Speech.cpp) on the
//! far end, and Qwen3-ASR 1.7B (llama.cpp) for the final pass of both sides. A real VAD is tested
//! on AMI audio, never on synthetic speech, which Silero scores as no speech.
//!
//! Ignored, and built only with the engine features:
//!
//! ```text
//! NEMO_SPEECH_DIR=<prefix> DYLD_LIBRARY_PATH=<prefix>/lib INK_BENCH_DIR=<bench> \
//!     cargo test -p ink-pipeline --release --features engine-silero,engine-nemo,engine-llama \
//!     --test real_engines -- --ignored --nocapture
//! ```
//!
//! `INK_BENCH_DIR` holds `models/silero-vad/silero_vad_16k_op15.onnx`,
//! `models/nemotron-3-diarization/` and `models/qwen3-asr-1.7b-gguf/`. The library path is needed
//! because ink-engines' build script gives NeMo's library an rpath only in its own binaries (on
//! Linux, `LD_LIBRARY_PATH`).
//!
//! Measured on an M5 Pro (2026-09-26): the final pass took 1.7 s; the mic gave 2 regions and 27
//! words, the far end 4 engine calls and 32 words; Nemotron found 4 substantial clusters in the far
//! end's 15 s of speech, where 3 people talk (the mix also carries the mic's talker through
//! headset crosstalk). It prints what it finds; the assertions are what must hold.

#![cfg(all(
    feature = "engine-silero",
    feature = "engine-nemo",
    feature = "engine-llama"
))]

mod meeting_rig;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ink_audio::{ChunkStore, FileReplaySource, SpeechProbability, capture_ring};
use ink_core::mock::{MemStore, MockClock};
use ink_core::{AudioSource, CancelToken, Channel, EventSink, Store, StreamFormat};
use ink_engines::llama::QwenAsr;
use ink_engines::{NemoDevice, NemoDiarizer, Registry, SileroModel, nemotron_3_diarization};
use ink_pipeline::capture::SideCapture;
use ink_pipeline::meeting::events::MeetingEvent;
use ink_pipeline::meeting::{MeetingChain, MeetingServices, MeetingStart};
use ink_pipeline::speech::VadSource;
use meeting_rig::*;

fn bench() -> PathBuf {
    PathBuf::from(std::env::var_os("INK_BENCH_DIR").expect("INK_BENCH_DIR is not set"))
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/ami")
}

fn qwen(bench: &Path) -> QwenAsr {
    let row = Registry::builtin()
        .unwrap()
        .get("qwen3-asr-1.7b-q8")
        .expect("the Qwen3-ASR row")
        .clone();
    let dir = bench.join("models/qwen3-asr-1.7b-gguf");
    QwenAsr::load(
        &dir.join(&row.files[0].name),
        &dir.join(&row.files[1].name),
        row.info(),
    )
    .expect("Qwen3-ASR loads")
}

fn nemo(bench: &Path) -> NemoDiarizer {
    let row = nemotron_3_diarization();
    NemoDiarizer::new(
        &bench
            .join("models/nemotron-3-diarization")
            .join(&row.files[0].name),
        row.info(),
        NemoDevice::Gpu(0),
    )
    .expect("the Nemotron model is installed")
}

#[test]
#[ignore = "needs the real engines' models under $INK_BENCH_DIR and NeMo's library; run locally"]
fn the_real_engines_on_the_ami_fixture() {
    let bench = bench();
    let silero = SileroModel::load(&bench.join("models/silero-vad/silero_vad_16k_op15.onnx"))
        .expect("the Silero model");
    let vad = VadSource::Installed(Arc::new(move || {
        Ok(Box::new(silero.vad()?) as Box<dyn SpeechProbability>)
    }));
    let asr = Arc::new(qwen(&bench));
    let diarizer = Arc::new(nemo(&bench));

    let clock = Arc::new(MockClock::new(T0_NS, T0_UNIX_MS));
    let dir = TempDir::new("real-engines");
    let chunks = ChunkStore::open(dir.path().join("record")).unwrap();
    let store = Arc::new(MemStore::new());
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink_events = events.clone();
    let sink: EventSink<MeetingEvent> = Arc::new(move |e| sink_events.lock().unwrap().push(e));
    let mut chain = MeetingChain::start(
        MeetingServices {
            // No live engine here: live partials come from FluidAudio in the Mac shell.
            live: None,
            offline: asr.clone(),
            diarizer: Some(diarizer.clone()),
            store: store.clone(),
            clock: clock.clone(),
            llm: None,
        },
        Default::default(),
        vad,
        sink,
        MeetingStart::default(),
    )
    .unwrap();
    let record = chain.record().clone();
    for (channel, file) in [
        (Channel::Mic, "IS1009a-mic.wav"),
        (Channel::Far, "IS1009a-far.wav"),
    ] {
        let (tx, rx) = capture_ring(StreamFormat::CANONICAL, Duration::from_secs(31)).unwrap();
        let mut source =
            FileReplaySource::open(fixtures().join(file), channel, clock.clone()).unwrap();
        source.start(Box::new(tx)).unwrap();
        source.wait().unwrap();
        let summary = SideCapture::new(channel, rx, chunks.clone()).finish(
            &mut |b| chain.push_audio(b.channel, &b.samples, b.host_time_ns, b.dropped_frames),
            &mut |i| panic!("capture: {i}"),
        );
        chain.capture_ended(summary);
    }
    let ended = chain.stop();
    let started = Instant::now();
    let outcome = ended.finalize(&chunks, &CancelToken::new()).unwrap();
    let took = started.elapsed();

    let finals = store.segments(&record).unwrap();
    let mut words_by_speaker: BTreeMap<String, usize> = BTreeMap::new();
    for s in finals.iter().filter(|s| s.channel == Channel::Far) {
        let who = s.speaker.as_ref().map_or("(none)".into(), |x| x.0.clone());
        *words_by_speaker.entry(who).or_default() += s.text.split_whitespace().count();
    }
    let warnings: Vec<_> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            MeetingEvent::Warning(w) => Some(format!("{w:?}")),
            _ => None,
        })
        .collect();
    println!("final pass in {took:.2?}");
    println!("mic: {:?}", outcome.mic);
    println!("far: {:?}", outcome.far);
    println!("diarization: {:?}", outcome.diarization);
    println!("far words by speaker: {words_by_speaker:?}");
    println!("warnings: {warnings:?}");

    assert_eq!(outcome.revision, Some(2));
    // You: the mic speaks for about 5 s of the 30; them: three people, most of the rest.
    assert!(outcome.mic.word_count > 0, "{:?}", outcome.mic);
    assert!(
        outcome.far.word_count > outcome.mic.word_count,
        "{:?}",
        outcome.far
    );
    assert_eq!(outcome.mic.failed_regions + outcome.far.failed_regions, 0);
    let d = outcome.diarization.expect("diarized");
    assert!(d.labelled && d.substantial >= 2, "{d:?}");
    assert!(
        finals
            .iter()
            .filter(|s| s.channel == Channel::Mic)
            .all(|s| s.speaker.is_none())
    );
    // Drop the engines before the process exits: ggml's Metal backend aborts an exit with a model
    // still loaded.
    drop(store);
    drop(asr);
    drop(diarizer);
}
