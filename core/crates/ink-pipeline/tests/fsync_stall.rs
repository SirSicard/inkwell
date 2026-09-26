//! Carry-over from S1.2a: the chunk writer syncs each finished chunk (F_FULLFSYNC on macOS) inside
//! `ChunkWriter::write`, on the pump. Does that stall capture?
//!
//! Capture stalls only if the ring overflows, and the ring holds
//! [`DEFAULT_RING_DURATION`](ink_audio::DEFAULT_RING_DURATION) (2 s). So the question is how long a
//! chunk-closing `write` takes on a real disk, against 2 s. This measures it: a meeting's worth of
//! far-end audio (48 kHz stereo, the heaviest stream) written in 10 ms blocks, as the pump writes
//! them, timing every call. It needs a real disk, so it is ignored in CI:
//!
//! ```text
//! cargo test -p ink-pipeline --test fsync_stall -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use ink_audio::{ChunkStore, DEFAULT_RING_DURATION};
use ink_core::{AudioBlock, Channel, StreamFormat};

const FORMAT: StreamFormat = StreamFormat {
    sample_rate: 48_000,
    channels: 2,
};
const BLOCK_FRAMES: usize = 480;

#[test]
#[ignore = "measures the disk: run locally with --ignored --nocapture"]
fn chunk_close_sync_on_the_pump_stays_far_under_the_ring() {
    let dir = std::env::temp_dir().join(format!("ink-fsync-stall-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = ChunkStore::open(&dir).expect("temp dir");
    let mut writer = store.writer(Channel::Far, FORMAT).expect("writer");

    // Ten minutes of audio: 60 chunk closes.
    let blocks = 10 * 60 * 100;
    let samples: Vec<f32> = (0..BLOCK_FRAMES * 2)
        .map(|i| ((i as f32) * 0.01).sin() * 0.1)
        .collect();
    let mut plain = Vec::with_capacity(blocks);
    let mut closing = Vec::new();
    for k in 0..blocks {
        let block = AudioBlock {
            samples: &samples,
            format: FORMAT,
            host_time_ns: k as u64 * 10_000_000,
        };
        let t = Instant::now();
        writer.write(&block, 0).expect("write");
        let took = t.elapsed();
        // A 10 s chunk holds 1000 blocks: the write that fills it closes and syncs it.
        if (k + 1) % 1000 == 0 {
            closing.push(took);
        } else {
            plain.push(took);
        }
    }
    let t = Instant::now();
    writer.finish().expect("finish");
    closing.push(t.elapsed());
    let _ = std::fs::remove_dir_all(&dir);

    let stats = |v: &mut Vec<Duration>| {
        v.sort_unstable();
        let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
        (at(0.5), at(0.99), *v.last().expect("some"))
    };
    let (c50, c99, cmax) = stats(&mut closing);
    let (p50, p99, pmax) = stats(&mut plain);
    println!(
        "chunk-closing writes ({}): p50 {c50:?}, p99 {c99:?}, max {cmax:?}",
        closing.len()
    );
    println!(
        "other writes ({}): p50 {p50:?}, p99 {p99:?}, max {pmax:?}",
        plain.len()
    );
    println!("ring: {DEFAULT_RING_DURATION:?}");
    // The pump drains both channels in turn, so one stall delays both rings. A tenth of the ring
    // leaves room for two closes back to back and a slow disk.
    assert!(
        cmax < DEFAULT_RING_DURATION / 10,
        "a chunk close took {cmax:?}: move the sync off the pump"
    );
}
