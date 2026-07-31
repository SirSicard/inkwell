//! Spike: can a streaming model show useful partial text while you speak?
//!
//! Parakeet is offline-only in sherpa-onnx, so live partials need a second
//! model running alongside it. The question this answers is not "does streaming
//! work" (it does) but whether the partials are good enough to put on screen
//! next to a final transcript the user will actually keep, and what that costs.
//!
//! Feeds a WAV through an OnlineRecognizer in real-time-sized chunks and prints
//! every partial as it changes, with the wall-clock time it appeared. Run it
//! against the same audio as the offline model to compare.
//!
//!     cargo run --release --example streaming_spike -- <model-dir> <wav>
//!
//! Model dir must hold encoder/decoder/joiner .onnx plus tokens.txt, e.g.
//! sherpa-onnx-streaming-zipformer-en-20M-2023-02-17.

use app_lib::filetranscribe;
use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn find(dir: &Path, needles: &[&str]) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut hits: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let n = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
            n.ends_with(".onnx") && needles.iter().all(|x| n.contains(x))
        })
        .collect();
    hits.sort();
    // Prefer int8 when both precisions are present: it is what a shipped
    // default would use, so the numbers should describe that.
    hits.iter().find(|p| p.to_string_lossy().contains("int8")).cloned().or_else(|| hits.first().cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: streaming_spike <model-dir> <wav>");
        std::process::exit(2);
    }
    let dir = PathBuf::from(&args[1]);
    let wav = PathBuf::from(&args[2]);

    let encoder = find(&dir, &["encoder"]).expect("no encoder .onnx in model dir");
    let decoder = find(&dir, &["decoder"]).expect("no decoder .onnx in model dir");
    let joiner = find(&dir, &["joiner"]).expect("no joiner .onnx in model dir");
    let tokens = dir.join("tokens.txt");
    assert!(tokens.exists(), "no tokens.txt in model dir");

    let mut config = OnlineRecognizerConfig::default();
    config.model_config.transducer.encoder = Some(encoder.to_string_lossy().into_owned());
    config.model_config.transducer.decoder = Some(decoder.to_string_lossy().into_owned());
    config.model_config.transducer.joiner = Some(joiner.to_string_lossy().into_owned());
    config.model_config.tokens = Some(tokens.to_string_lossy().into_owned());
    config.model_config.num_threads = 2; // leave cores for the offline pass
    config.model_config.provider = Some("cpu".into());
    config.decoding_method = Some("greedy_search".into());
    config.enable_endpoint = true;

    let load_started = Instant::now();
    let recognizer = OnlineRecognizer::create(&config).expect("failed to create recognizer");
    println!("model loaded in {} ms", load_started.elapsed().as_millis());

    let samples = filetranscribe::decode_to_pcm(&wav).expect("decode failed");
    let audio_secs = samples.len() as f32 / 16000.0;
    println!("audio: {:.1}s\n", audio_secs);

    // 100ms blocks, fed as fast as the CPU allows. Real-time pacing would only
    // measure sleep(); what matters is whether decoding keeps up with speech,
    // which is the ratio at the end.
    const BLOCK: usize = 1600;
    let stream = recognizer.create_stream();
    let started = Instant::now();
    let mut last = String::new();
    let mut first_partial_ms: Option<u128> = None;

    for (i, block) in samples.chunks(BLOCK).enumerate() {
        stream.accept_waveform(16000, block);
        while recognizer.is_ready(&stream) {
            recognizer.decode(&stream);
        }
        if let Some(r) = recognizer.get_result(&stream) {
            let text = r.text.trim().to_string();
            if !text.is_empty() && text != last {
                let at = started.elapsed().as_millis();
                first_partial_ms.get_or_insert(at);
                // Audio position vs when the text appeared: the gap is the lag
                // a user would perceive if this were on screen.
                let audio_ms = ((i + 1) * BLOCK) as f32 / 16.0;
                println!("[audio {:>6.0}ms | cpu {:>6}ms] {}", audio_ms, at, text);
                last = text;
            }
        }
    }

    stream.input_finished();
    while recognizer.is_ready(&stream) {
        recognizer.decode(&stream);
    }
    let total = started.elapsed();
    let final_text = recognizer.get_result(&stream).map(|r| r.text.trim().to_string()).unwrap_or_default();

    println!("\nstreaming final : {:?}", final_text);
    println!(
        "decode time     : {} ms for {:.1}s of audio (RTF {:.3}, {:.0}x real time)",
        total.as_millis(),
        audio_secs,
        total.as_secs_f32() / audio_secs,
        audio_secs / total.as_secs_f32()
    );
    if let Some(ms) = first_partial_ms {
        println!("first partial   : {} ms in", ms);
    }
    println!(
        "\nCompare against the offline model on the same file with:\n  \
         cargo run --release --example ab_models -- --corpus <dir containing this wav>"
    );
}
