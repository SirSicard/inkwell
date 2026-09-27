//! The final pass's time for a long meeting (S2.8's Verify: "finalize time for 60 minutes within
//! S0.6's target"), on the real engines: Silero VAD, Nemotron-3-Diarization on the far end,
//! Qwen3-ASR 1.7B for both sides. A measurement, not a check: it is ignored, built only with the
//! engine features, and asserts the target only when `INK_FINALIZE_ASSERT=1`.
//!
//! The meeting is the committed AMI fixture (IS1009a, 30 s a side, public), looped to
//! `INK_FINALIZE_MINUTES` (60 by default) and written straight into chunks on disk, as a meeting's
//! pump writes them, then finalized as a meeting interrupted at its end
//! (`EndedMeeting::interrupted`): the same final pass a stopped meeting runs, without first
//! replaying an hour in real time. The summary is left out (its model, Foundation Models, runs in
//! the Mac shell); the dogfood week times it on real meetings.
//!
//! The target: S0.6 names none in minutes; it rests on the gate's speed for the meeting final,
//! Qwen3-ASR at about 25x real time ("a 90-minute meeting takes about 3.5 minutes", per side),
//! so 60 minutes of both sides is at most 2 x 60 / 25 = 4.8 minutes of ASR, the diarizer (343x)
//! adding about 10 s. `INK_FINALIZE_TARGET_S` overrides the 288 s. Speech-only regions make the
//! real pass shorter than that bound on a meeting with pauses; the looped fixture has few.
//!
//! Run it on an otherwise idle Mac (mac/scripts/finalize-time.sh does, and says so):
//!
//! ```text
//! NEMO_SPEECH_DIR=<prefix> INK_BENCH_DIR=<bench> \
//!     cargo test -p ink-pipeline --release --features engine-silero,engine-nemo,engine-llama \
//!     --test finalize_time -- --ignored --nocapture
//! ```
//!
//! A row goes to `$INK_BENCH_DIR/out/s2.8/finalize-time.tsv` (never into the repository).

#![cfg(all(
    feature = "engine-silero",
    feature = "engine-nemo",
    feature = "engine-llama"
))]

mod meeting_rig;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ink_audio::{ChunkStore, SpeechProbability};
use ink_core::mock::{MemStore, MockClock};
use ink_core::{
    AudioBlock, CancelToken, Channel, EventSink, NewRecord, RecordKind, Store, StreamFormat,
};
use ink_engines::llama::QwenAsr;
use ink_engines::{NemoDevice, NemoDiarizer, Registry, SileroModel, nemotron_3_diarization};
use ink_pipeline::meeting::events::MeetingEvent;
use ink_pipeline::meeting::{EndedMeeting, Interrupted, MeetingServices};
use ink_pipeline::speech::VadSource;
use meeting_rig::*;

fn bench() -> PathBuf {
    PathBuf::from(std::env::var_os("INK_BENCH_DIR").expect("INK_BENCH_DIR is not set"))
}

fn env_f64(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn fixture(name: &str) -> Vec<f32> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/ami")
        .join(name);
    let mut reader = hound::WavReader::open(path).unwrap();
    reader
        .samples::<i32>()
        .map(|s| s.unwrap() as f32 / 32_768.0)
        .collect()
}

/// Writes `minutes` of `source`, looped, as `channel`'s chunks, in 20 ms blocks stamped from `t0`.
fn write_side(chunks: &ChunkStore, channel: Channel, source: &[f32], minutes: f64, t0: u64) {
    let format = StreamFormat::CANONICAL;
    let total = (minutes * 60.0 * 16_000.0) as usize;
    let mut writer = chunks.writer(channel, format).unwrap();
    let block = 320;
    let mut at = 0usize;
    let mut buffer = vec![0.0f32; block];
    while at < total {
        let n = block.min(total - at);
        for (i, sample) in buffer[..n].iter_mut().enumerate() {
            *sample = source[(at + i) % source.len()];
        }
        writer
            .write(
                &AudioBlock {
                    samples: &buffer[..n],
                    format,
                    host_time_ns: t0 + (at as u64) * 1_000_000_000 / 16_000,
                },
                0,
            )
            .unwrap();
        at += n;
    }
    writer.finish().unwrap();
}

#[test]
#[ignore = "a measurement on the real engines: models under $INK_BENCH_DIR, NeMo's library; run on an idle Mac"]
fn the_final_pass_of_a_long_meeting_is_timed() {
    let bench = bench();
    let minutes = env_f64("INK_FINALIZE_MINUTES", 60.0);
    let target_s = env_f64("INK_FINALIZE_TARGET_S", 2.0 * minutes * 60.0 / 25.0);

    let silero = SileroModel::load(&bench.join("models/silero-vad/silero_vad_16k_op15.onnx"))
        .expect("the Silero model");
    let vad = VadSource::Installed(Arc::new(move || {
        Ok(Box::new(silero.vad()?) as Box<dyn SpeechProbability>)
    }));
    let row = Registry::builtin()
        .unwrap()
        .get("qwen3-asr-1.7b-q8")
        .expect("the Qwen3-ASR row")
        .clone();
    let qwen_dir = bench.join("models/qwen3-asr-1.7b-gguf");
    let asr = Arc::new(
        QwenAsr::load(
            &qwen_dir.join(&row.files[0].name),
            &qwen_dir.join(&row.files[1].name),
            row.info(),
        )
        .expect("Qwen3-ASR loads"),
    );
    let diar_row = nemotron_3_diarization();
    let diarizer = Arc::new(
        NemoDiarizer::new(
            &bench
                .join("models/nemotron-3-diarization")
                .join(&diar_row.files[0].name),
            diar_row.info(),
            NemoDevice::Gpu(0),
        )
        .expect("the Nemotron model"),
    );

    let dir = TempDir::new("finalize-time");
    let chunks = ChunkStore::open(dir.path().join("record")).unwrap();
    let written = Instant::now();
    write_side(
        &chunks,
        Channel::Mic,
        &fixture("IS1009a-mic.wav"),
        minutes,
        T0_NS,
    );
    write_side(
        &chunks,
        Channel::Far,
        &fixture("IS1009a-far.wav"),
        minutes,
        T0_NS,
    );
    let wrote = written.elapsed();

    let store = Arc::new(MemStore::new());
    let record = store
        .create_record(NewRecord {
            kind: RecordKind::Meeting,
            title: Some("Finalize time".into()),
            started_at_unix_ms: T0_UNIX_MS,
            source_app: None,
            audio_dir: Some("record".into()),
        })
        .unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink_events = events.clone();
    let sink: EventSink<MeetingEvent> = Arc::new(move |e| sink_events.lock().unwrap().push(e));
    let ended = EndedMeeting::interrupted(
        MeetingServices {
            live: None,
            offline: asr,
            diarizer: Some(diarizer),
            store: store.clone(),
            clock: Arc::new(MockClock::new(T0_NS, T0_UNIX_MS)),
            llm: None,
        },
        Default::default(),
        vad,
        sink,
        Interrupted {
            record: record.clone(),
            started_unix_ms: T0_UNIX_MS,
            t0_ns: T0_NS,
            ended_unix_ms: T0_UNIX_MS + (minutes * 60_000.0) as i64,
        },
    );
    let started = Instant::now();
    let outcome = ended.finalize(&chunks, &CancelToken::new()).unwrap();
    let took = started.elapsed().as_secs_f64();

    let words = store
        .segments(&record)
        .unwrap()
        .iter()
        .map(|s| s.text.split_whitespace().count())
        .sum::<usize>();
    let line = format!(
        "{minutes:.1}\t{took:.1}\t{target_s:.1}\t{:.1}\t{}\t{}\t{}\t{words}\t{:.1}",
        minutes * 60.0 / took,
        outcome.mic.regions,
        outcome.far.regions,
        outcome.diarization.map_or(0, |d| d.clusters),
        wrote.as_secs_f64()
    );
    eprintln!(
        "finalize time: {minutes:.1} min of meeting (both sides) in {took:.1} s \
         ({:.1}x real time); target {target_s:.0} s; regions mic {} far {}; {words} words; \
         chunks written in {:.1} s",
        minutes * 60.0 / took,
        outcome.mic.regions,
        outcome.far.regions,
        wrote.as_secs_f64()
    );
    let out = bench.join("out/s2.8");
    std::fs::create_dir_all(&out).unwrap();
    let path = out.join("finalize-time.tsv");
    let new = !path.exists();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .unwrap();
    if new {
        writeln!(
            file,
            "minutes\ttook_s\ttarget_s\tx_real_time\tmic_regions\tfar_regions\tclusters\twords\twrite_s"
        )
        .unwrap();
    }
    writeln!(file, "{line}").unwrap();
    assert!(
        outcome.superseded,
        "the pass replaced the (empty) live transcript"
    );
    if std::env::var("INK_FINALIZE_ASSERT").as_deref() == Ok("1") {
        assert!(
            took <= target_s,
            "{took:.1} s for {minutes} minutes: over the {target_s:.0} s target"
        );
    }
}
