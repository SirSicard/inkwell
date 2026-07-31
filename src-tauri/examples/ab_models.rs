//! Compare transcription models on real recordings, with a number at the end.
//!
//! Model changes are otherwise argued from vibes: a new model "feels" better
//! because the one sentence you tried came out right. This runs every installed
//! candidate over the same corpus and reports word error rate against what you
//! actually said, so a change either wins or it does not.
//!
//! Build a corpus by turning on Save Debug Audio (General > Advanced) and
//! dictating. Each take lands in ~/Documents/Inkwell Debug Audio as take-NNNN.wav.
//! Then write the truth: a take-NNNN.txt beside each wav containing exactly what
//! you said. Takes with no .txt are still transcribed (so you can eyeball them)
//! but do not count toward WER.
//!
//!     cargo run --release --example ab_models
//!     cargo run --release --example ab_models -- --models parakeet,parakeet-v2
//!     cargo run --release --example ab_models -- --corpus /path/to/wavs
//!
//! Hotwords come from the app's own dictionary.json, so the comparison includes
//! the biasing the real pipeline applies rather than measuring a configuration
//! nobody runs.

use app_lib::{dictionary, filetranscribe, models};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME not set"))
}

fn app_data() -> PathBuf {
    home().join("Library/Application Support/com.inkwell.app")
}

/// Levenshtein distance over words, the standard WER numerator.
fn word_distance(reference: &[String], hypothesis: &[String]) -> usize {
    let mut prev: Vec<usize> = (0..=hypothesis.len()).collect();
    let mut cur = vec![0usize; hypothesis.len() + 1];
    for (i, r) in reference.iter().enumerate() {
        cur[0] = i + 1;
        for (j, h) in hypothesis.iter().enumerate() {
            let cost = if r == h { 0 } else { 1 };
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[hypothesis.len()]
}

/// Case and punctuation are style, not accuracy: the pipeline applies its own
/// casing and punctuation afterwards, so scoring them here would measure the
/// wrong thing and hide real word errors behind formatting noise.
fn normalize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '\'' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

struct Sample {
    name: String,
    audio: Vec<f32>,
    truth: Option<Vec<String>>,
}

fn load_corpus(dir: &Path) -> Vec<Sample> {
    let mut entries: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "wav"))
            .collect(),
        Err(e) => {
            eprintln!("Cannot read corpus dir {}: {}", dir.display(), e);
            return Vec::new();
        }
    };
    entries.sort();

    entries
        .into_iter()
        .filter_map(|wav| {
            let audio = match filetranscribe::decode_to_pcm(&wav) {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("  skipping {}: {}", wav.display(), e);
                    return None;
                }
            };
            let truth = std::fs::read_to_string(wav.with_extension("txt"))
                .ok()
                .map(|t| normalize(&t));
            Some(Sample {
                name: wav.file_stem().unwrap_or_default().to_string_lossy().into_owned(),
                audio,
                truth,
            })
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |flag: &str| -> Option<String> {
        args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
    };

    let corpus_dir = arg("--corpus")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join("Documents/Inkwell Debug Audio"));
    let models_dir = app_data().join("models");

    let corpus = load_corpus(&corpus_dir);
    if corpus.is_empty() {
        eprintln!(
            "No .wav files in {}.\nTurn on Save Debug Audio in General > Advanced, dictate a few \
             sentences, then write what you said into take-NNNN.txt beside each recording.",
            corpus_dir.display()
        );
        std::process::exit(1);
    }
    let scored = corpus.iter().filter(|s| s.truth.is_some()).count();
    println!(
        "Corpus: {} recordings from {} ({} with a .txt to score against)\n",
        corpus.len(),
        corpus_dir.display(),
        scored
    );

    // Bias with the same dictionary the app uses, so this measures the shipped
    // configuration rather than a bare recognizer nobody runs.
    let dict = dictionary::Dictionary::load(&app_data().join("dictionary.json"));
    let hotwords = dict.hotwords();
    match &hotwords {
        Some(h) => println!("Hotwords: {}\n", h.replace('\n', ", ")),
        None => println!("Hotwords: none (dictionary is empty)\n"),
    }

    let wanted: Option<Vec<String>> = arg("--models")
        .map(|m| m.split(',').map(|s| s.trim().to_string()).collect());

    let candidates: Vec<&models::ModelSpec> = models::MODELS
        .iter()
        .filter(|m| wanted.as_ref().is_none_or(|w| w.iter().any(|x| x == m.id)))
        .filter(|m| {
            let installed = m.is_installed(&models_dir);
            if !installed && wanted.is_some() {
                println!("  {} is not installed, skipping", m.id);
            }
            installed
        })
        .collect();

    if candidates.is_empty() {
        eprintln!("No installed models to compare. Download one in the Models tab first.");
        std::process::exit(1);
    }

    let mut results: Vec<(String, f64, u128, usize)> = Vec::new();
    let mut transcripts: HashMap<String, Vec<String>> = HashMap::new();

    for spec in &candidates {
        print!("Loading {}... ", spec.display);
        use std::io::Write;
        let _ = std::io::stdout().flush();

        let eng = match spec.load(&models_dir) {
            Ok(e) => e,
            Err(e) => {
                println!("FAILED: {}", e);
                continue;
            }
        };
        println!("ok");

        let mut errors = 0usize;
        let mut ref_words = 0usize;
        let mut elapsed_ms = 0u128;
        let mut texts = Vec::new();

        for sample in &corpus {
            let started = std::time::Instant::now();
            let text = match eng.transcribe(&sample.audio, hotwords.as_deref()) {
                Ok(t) => t,
                Err(e) => {
                    println!("  {}: ERROR {}", sample.name, e);
                    continue;
                }
            };
            elapsed_ms += started.elapsed().as_millis();

            let hyp = normalize(&text);
            let marker = match &sample.truth {
                Some(truth) => {
                    let d = word_distance(truth, &hyp);
                    errors += d;
                    ref_words += truth.len();
                    if d == 0 { "OK  ".to_string() } else { format!("{:<4}", d) }
                }
                None => "--  ".to_string(),
            };
            println!("  {} {}: {}", marker, sample.name, text);
            texts.push(text);
        }

        let wer = if ref_words > 0 {
            100.0 * errors as f64 / ref_words as f64
        } else {
            f64::NAN
        };
        println!();
        results.push((spec.display.to_string(), wer, elapsed_ms, ref_words));
        transcripts.insert(spec.display.to_string(), texts);
    }

    println!("\n{:<32} {:>9} {:>12}", "MODEL", "WER", "TOTAL TIME");
    println!("{}", "-".repeat(55));
    let mut ranked = results.clone();
    ranked.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    for (name, wer, ms, refs) in &ranked {
        let wer_s = if refs > &0 { format!("{:.1}%", wer) } else { "n/a".to_string() };
        println!("{:<32} {:>9} {:>10}ms", name, wer_s, ms);
    }

    if scored == 0 {
        println!(
            "\nNo .txt truth files, so WER is unavailable and the ranking above is by nothing.\n\
             Write what you actually said into take-NNNN.txt beside each recording to get a score."
        );
    } else if ranked.len() > 1 {
        let (best, best_wer, _, _) = &ranked[0];
        println!("\nBest on this corpus: {} at {:.1}% WER.", best, best_wer);
        println!(
            "Judge on {} scored recordings; a difference of a word or two is noise, not a result.",
            scored
        );
    }
}
