//! Does live preview actually produce readable partials?
//!
//! The spike (`streaming_spike.rs`) answered "is a streaming model fast enough
//! and good enough" against a config written for the measurement. This answers
//! a narrower and later question: does the code that ships do it, through the
//! resampler that ships, at a capture rate a real microphone actually uses.
//!
//! It exists because the feature cannot be tested any other way without a
//! person and a microphone. Everything up to the moment audio arrives is
//! covered here; what is left for a human is whether it feels right.
//!
//!     cargo run --release --example streaming_check -- <model-dir> <wav> [rate]
//!
//! `rate` is the capture rate to pretend the file arrived at, so the
//! `StreamResampler` is exercised rather than bypassed. Defaults to 48000,
//! which is what most machines capture at.

use app_lib::filetranscribe;
use app_lib::streaming::{display_partial, StreamResampler};
use std::path::PathBuf;
use std::time::Instant;

/// Same 100 ms cadence as `pipeline::PARTIAL_FEED_INTERVAL_MS`, so the chunk
/// sizes the resampler sees here are the sizes it sees in the app.
const FEED_MS: usize = 100;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: streaming_check <model-dir> <wav> [capture-rate]");
        std::process::exit(2);
    }
    let dir = PathBuf::from(&args[1]);
    let wav = PathBuf::from(&args[2]);
    let capture_rate: usize = args
        .get(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(48_000);

    let load_started = Instant::now();
    let recognizer = match app_lib::streaming::build(&dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("build failed: {}", e);
            std::process::exit(1);
        }
    };
    println!("model loaded in {} ms", load_started.elapsed().as_millis());

    // The file is 16 kHz; upsample it so the run starts from a capture rate a
    // microphone would produce and the resampler has real work to do.
    let file_pcm = filetranscribe::decode_to_pcm(&wav).expect("decode failed");
    let source: Vec<f32> = if capture_rate == 16_000 {
        file_pcm.clone()
    } else {
        let ratio = capture_rate as f64 / 16_000.0;
        let n = (file_pcm.len() as f64 * ratio) as usize;
        (0..n)
            .map(|i| {
                let p = i as f64 / ratio;
                let a = p.floor() as usize;
                let f = (p - a as f64) as f32;
                let s0 = file_pcm.get(a).copied().unwrap_or(0.0);
                let s1 = file_pcm.get(a + 1).copied().unwrap_or(s0);
                s0 + (s1 - s0) * f
            })
            .collect()
    };
    let audio_secs = file_pcm.len() as f32 / 16_000.0;
    println!(
        "audio: {:.1}s, fed at {} Hz in {} ms chunks\n",
        audio_secs, capture_rate, FEED_MS
    );

    let mut resampler = StreamResampler::new(capture_rate);
    let stream = recognizer.create_stream();
    let started = Instant::now();
    let mut last = String::new();
    let mut first_partial_ms: Option<u128> = None;
    let mut resampled_total = 0usize;

    let block = capture_rate * FEED_MS / 1000;
    for (i, chunk) in source.chunks(block).enumerate() {
        let pcm = resampler.push(chunk);
        resampled_total += pcm.len();
        if pcm.is_empty() {
            continue;
        }
        stream.accept_waveform(16_000, &pcm);
        while recognizer.is_ready(&stream) {
            recognizer.decode(&stream);
        }
        if let Some(r) = recognizer.get_result(&stream) {
            let text = display_partial(&r.text, 96);
            if !text.is_empty() && text != last {
                let at = started.elapsed().as_millis();
                first_partial_ms.get_or_insert(at);
                let audio_ms = ((i + 1) * FEED_MS) as f32;
                println!("[audio {:>6.0}ms | cpu {:>5}ms] {}", audio_ms, at, text);
                last = text;
            }
        }
    }

    stream.input_finished();
    while recognizer.is_ready(&stream) {
        recognizer.decode(&stream);
    }
    let total = started.elapsed();
    let final_raw = recognizer
        .get_result(&stream)
        .map(|r| r.text.trim().to_string())
        .unwrap_or_default();

    println!("\nlast partial shown : {:?}", last);
    println!("raw decoder output : {:?}", final_raw);
    println!(
        "resampled          : {} samples for {:.1}s, expected ~{}",
        resampled_total,
        audio_secs,
        (audio_secs * 16_000.0) as usize
    );
    println!(
        "decode time        : {} ms for {:.1}s of audio ({:.0}x real time)",
        total.as_millis(),
        audio_secs,
        audio_secs / total.as_secs_f32()
    );
    if let Some(ms) = first_partial_ms {
        println!("first partial      : {} ms in", ms);
    }

    // The two things that would make this unusable in the app, as opposed to
    // merely imperfect, are silence and drift. Everything else is the known
    // cost of streaming and is what the offline pass exists to fix.
    let drift = (resampled_total as f32 - audio_secs * 16_000.0).abs() / (audio_secs * 16_000.0);
    if last.is_empty() {
        eprintln!("\nFAIL: no partial was ever produced");
        std::process::exit(1);
    }
    if drift > 0.01 {
        eprintln!("\nFAIL: resampler drifted {:.2}% from real time", drift * 100.0);
        std::process::exit(1);
    }
    println!("\nOK: partials produced, resampler within {:.3}% of real time", drift * 100.0);
}
