//! Bands: one FFT per hop into three energy bands, read out through a copy-out reader.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use ink_audio::bands::{
    BandAnalyzer, Bands, BandsReader, FFT_LEN, HIGH_HZ, HOP, LOW_HZ, MID_HZ, bands_channel,
};
use ink_audio::gain::to_dbfs;
use ink_audio::synth::{speech_like, tone};

/// Runs `audio` through a fresh analyzer and returns the last hop's bands.
fn analyse(audio: &[f32]) -> Bands {
    let mut a = BandAnalyzer::new();
    a.process(audio).expect("at least one hop")
}

#[test]
fn silence_gives_zero_bands() {
    let b = analyse(&vec![0.0; 16_000]);
    assert_eq!(b, Bands::default());
    assert_eq!((b.low, b.mid, b.high), (0.0, 0.0, 0.0));
}

#[test]
fn the_band_edges_are_the_documented_ones() {
    assert_eq!((LOW_HZ.start, LOW_HZ.end), (80.0, 500.0));
    assert_eq!((MID_HZ.start, MID_HZ.end), (500.0, 2_000.0));
    assert_eq!((HIGH_HZ.start, HIGH_HZ.end), (2_000.0, 8_000.0));
    assert_eq!((FFT_LEN, HOP), (512, 256));
}

#[test]
fn a_tone_lights_its_own_band_at_its_level() {
    // A 0.5-peak sine has an RMS of 0.354 (−9 dBFS); its band reads that, within 0.5 dB, and the
    // other two stay at least 40 dB below it (Hann leakage).
    let want = to_dbfs(0.5 / 2f32.sqrt());
    for (hz, band) in [(250.0, 0usize), (1_000.0, 1), (4_000.0, 2)] {
        let b = analyse(&tone(0.5, hz, 0.5, 16_000));
        let values = [b.low, b.mid, b.high];
        let level = to_dbfs(values[band]);
        assert!(
            (level - want).abs() < 0.5,
            "{hz} Hz read {level:.2} dBFS in its band, want {want:.2}"
        );
        for (other, &v) in values.iter().enumerate() {
            if other != band {
                assert!(
                    to_dbfs(v) < want - 40.0,
                    "{hz} Hz leaked into band {other}: {:.1} dBFS",
                    to_dbfs(v)
                );
            }
        }
    }
}

#[test]
fn speech_lights_all_three_bands() {
    let b = analyse(&speech_like(1.0, -20.0, 1));
    assert!(b.low > 0.0 && b.mid > 0.0 && b.high > 0.0, "{b:?}");
}

#[test]
fn one_fft_per_hop_whatever_the_push_size() {
    let audio = speech_like(1.0, -20.0, 2);
    let mut whole = BandAnalyzer::new();
    let last_whole = whole.process(&audio);
    assert_eq!(whole.hops(), (audio.len() / HOP) as u64);

    let mut pieces = BandAnalyzer::new();
    let mut last = None;
    for chunk in audio.chunks(97) {
        if let Some(b) = pieces.process(chunk) {
            last = Some(b);
        }
    }
    assert_eq!(pieces.hops(), whole.hops());
    // Same hops over the same samples: the same bands (allowed: 1e-6 relative).
    let (a, b) = (last_whole.unwrap(), last.unwrap());
    for (x, y) in [(a.low, b.low), (a.mid, b.mid), (a.high, b.high)] {
        assert!((x - y).abs() <= 1e-6 * x.abs().max(1e-12), "{a:?} vs {b:?}");
    }
    // Less than a hop in: no new bands yet.
    let mut fresh = BandAnalyzer::new();
    assert_eq!(fresh.process(&audio[..HOP - 1]), None);
    assert!(fresh.process(&audio[HOP - 1..HOP]).is_some());
}

#[test]
fn a_reader_copies_out_the_latest_published_bands() {
    let (mut writer, reader) = bands_channel();
    let first = reader.read();
    assert_eq!(first.bands, Bands::default());
    assert_eq!(first.published, 0);
    let b = Bands {
        low: 0.1,
        mid: 0.2,
        high: 0.3,
    };
    writer.publish(b);
    let copy = reader.clone().read();
    assert_eq!(copy.bands, b);
    assert_eq!(copy.published, 1);
}

#[test]
fn readers_are_shareable_across_threads() {
    fn shareable<T: Send + Sync + Clone + 'static>() {}
    shareable::<BandsReader>();
    fn movable<T: Send + 'static>() {}
    movable::<ink_audio::bands::BandsWriter>();
    movable::<BandAnalyzer>();
}

#[test]
fn the_reader_never_sees_a_torn_value() {
    // The writer publishes (n, n, n) as its n-th value while readers copy out as fast as they
    // can. A torn read would mix two publishes: bands that disagree with each other or with the
    // publish count they came with.
    const PUBLISHES: u32 = 300_000;
    let (mut writer, reader) = bands_channel();
    let done = Arc::new(AtomicBool::new(false));
    let readers: Vec<_> = (0..3)
        .map(|_| {
            let reader = reader.clone();
            let done = done.clone();
            thread::spawn(move || {
                let mut reads = 0u64;
                let mut last = 0u64;
                loop {
                    let finished = done.load(Ordering::Acquire);
                    let snap = reader.read();
                    let n = snap.published as f32;
                    assert_eq!(
                        (snap.bands.low, snap.bands.mid, snap.bands.high),
                        (n, n, n),
                        "torn read"
                    );
                    assert!(snap.published >= last, "went backwards");
                    last = snap.published;
                    reads += 1;
                    if finished {
                        return (reads, last);
                    }
                }
            })
        })
        .collect();
    for n in 1..=PUBLISHES {
        let v = n as f32;
        writer.publish(Bands {
            low: v,
            mid: v,
            high: v,
        });
    }
    done.store(true, Ordering::Release);
    for r in readers {
        let (reads, last) = r.join().expect("reader thread");
        assert!(reads > 0);
        assert_eq!(last, u64::from(PUBLISHES), "saw the final publish");
    }
}

/// The ink's level map (mac/Sources/InkRenderer/InkLevels.swift: `floorDB`, `ceilingDB`) rests
/// on these measurements of the public AMI fixture as published (S2.8): every 16 ms hop's band
/// power, in dBFS, as the pump publishes it for the ink.
///
/// | Fixture | p10 | p25 | p50 | p75 | p90 | p99 |
/// |---|---|---|---|---|---|---|
/// | `IS1009a-mic.wav` (one headset: the talker and the room between words) | -58.8 | -56.6 | -50.6 | -40.9 | -25.1 | -19.4 |
/// | `IS1009a-far.wav` (three headsets mixed: nearly always someone talking) | -53.0 | -50.6 | -42.1 | -34.8 | -28.6 | -18.4 |
///
/// So a floor of -55 dBFS leaves the room between words still (the mic's p10 and p25 read 0;
/// S0.3's built-in-mic room floor, -50.6 dB full band, reads about 0.1: barely a breath), and a
/// ceiling of -20 dBFS lets only the loudest speech saturate (p99 -19.4). The droplet threshold,
/// 0.5, falls at -37.5 dBFS: the far mix crosses it from its p75, the single talker in its loud
/// quarter. The same map serves both sides: the far end's digital silence reads 0 anyway. This
/// test keeps the table true; the Swift constants cite it.
#[test]
fn the_ink_level_map_rests_on_the_measured_fixture_levels() {
    const FLOOR_DB: f64 = -55.0;
    const CEILING_DB: f64 = -20.0;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/ami");
    let percentiles = |file: &str| -> Vec<f64> {
        let mut reader = hound::WavReader::open(root.join(file)).unwrap();
        let samples: Vec<f32> = reader
            .samples::<i32>()
            .map(|x| x.unwrap() as f32 / 32_768.0)
            .collect();
        let mut analyzer = BandAnalyzer::new();
        let mut db: Vec<f64> = samples
            .chunks(HOP)
            .filter_map(|c| analyzer.process(c))
            .map(|b| {
                let p = f64::from(b.low * b.low + b.mid * b.mid + b.high * b.high);
                if p > 0.0 { 10.0 * p.log10() } else { -150.0 }
            })
            .collect();
        db.sort_by(f64::total_cmp);
        [0.10, 0.25, 0.50, 0.75, 0.90, 0.99]
            .iter()
            .map(|q| db[((db.len() - 1) as f64 * q) as usize])
            .collect()
    };
    let level = |db: f64| ((db - FLOOR_DB) / (CEILING_DB - FLOOR_DB)).clamp(0.0, 1.0);
    let near = percentiles("IS1009a-mic.wav");
    let far = percentiles("IS1009a-far.wav");
    let table = [
        (&near, [-58.8, -56.6, -50.6, -40.9, -25.1, -19.4]),
        (&far, [-53.0, -50.6, -42.1, -34.8, -28.6, -18.4]),
    ];
    for (measured, documented) in table {
        for (m, d) in measured.iter().zip(documented) {
            assert!(
                (m - d).abs() < 0.05,
                "measured {measured:?}, documented {documented:?}"
            );
        }
    }
    assert_eq!(level(near[0]), 0.0, "the room between words is still");
    assert_eq!(level(near[1]), 0.0);
    assert!(
        level(near[4]) > 0.5,
        "a talker's loud quarter throws droplets"
    );
    assert!(level(far[3]) > 0.5 && level(far[2]) < 0.5);
    assert!(
        level(near[4]) < 1.0 && level(far[4]) < 1.0,
        "only the loudest speech saturates"
    );
    assert!(level(-50.6) < 0.15, "S0.3's room floor barely moves it");
}
