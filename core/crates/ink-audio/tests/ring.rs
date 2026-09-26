//! The capture ring: order, timestamps, format, sizing, and overruns that are never silent.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{MONO_16K, STEREO_48K, signal};
use ink_audio::{
    CaptureConsumer, CapturedBlock, DEFAULT_RING_DURATION, Overruns, RingError, capture_ring,
};
use ink_core::mock::MockPlatform;
use ink_core::{AudioBlock, AudioSink, Channel, StreamFormat};

fn block(samples: &[f32], format: StreamFormat, host_time_ns: u64) -> AudioBlock<'_> {
    AudioBlock {
        samples,
        format,
        host_time_ns,
    }
}

/// Pops everything as owned (samples, host time, format, dropped blocks, dropped frames).
fn drain(consumer: &mut CaptureConsumer) -> Vec<(Vec<f32>, u64, StreamFormat, u64, u64)> {
    let mut out = Vec::new();
    while let Some(c) = consumer.pop() {
        out.push((
            c.block.samples.to_vec(),
            c.block.host_time_ns,
            c.block.format,
            c.dropped_blocks_before,
            c.dropped_frames_before,
        ));
    }
    out
}

#[test]
fn ring_returns_blocks_in_order_with_timestamps_and_format() {
    let (mut producer, mut consumer) = capture_ring(STEREO_48K, DEFAULT_RING_DURATION).unwrap();
    let audio = signal(4_800, STEREO_48K, 1);
    // Uneven block sizes, as some drivers deliver.
    let sizes = [480usize, 512, 1_000, 7, 2_801];
    let mut frame = 0usize;
    let mut expected = Vec::new();
    for (i, &n) in sizes.iter().enumerate() {
        let samples = &audio[frame * 2..(frame + n) * 2];
        let t = 1_000_000 + i as u64 * 10_000_000;
        producer.push(&block(samples, STEREO_48K, t));
        expected.push((samples.to_vec(), t, STEREO_48K, 0, 0));
        frame += n;
    }
    assert_eq!(drain(&mut consumer), expected);
    assert!(consumer.pop().is_none());
    assert_eq!(consumer.overruns(), Overruns::default());
}

#[test]
fn ring_is_sized_from_a_duration() {
    // Two seconds at 16 kHz mono: exactly 100 blocks of 20 ms fit, the 101st overruns.
    let (mut producer, mut consumer) = capture_ring(MONO_16K, Duration::from_secs(2)).unwrap();
    let audio = signal(320, MONO_16K, 2);
    for i in 0..100 {
        producer.push(&block(&audio, MONO_16K, i * 20_000_000));
    }
    assert_eq!(producer.overruns(), Overruns::default(), "2 s fits");
    producer.push(&block(&audio, MONO_16K, 100 * 20_000_000));
    assert_eq!(
        producer.overruns(),
        Overruns {
            blocks: 1,
            frames: 320
        }
    );
    assert_eq!(drain(&mut consumer).len(), 100);

    // Stereo at 48 kHz: the ring holds frames, not samples, for the duration.
    let (mut producer, _consumer) = capture_ring(STEREO_48K, Duration::from_secs(1)).unwrap();
    let audio = signal(48_000, STEREO_48K, 3);
    producer.push(&block(&audio, STEREO_48K, 0));
    assert_eq!(
        producer.overruns(),
        Overruns::default(),
        "1 s of stereo fits"
    );
}

#[test]
fn overrun_drops_the_block_counts_it_and_reports_the_gap_where_it_happened() {
    let (mut producer, mut consumer) = capture_ring(MONO_16K, Duration::from_millis(100)).unwrap();
    let audio = signal(1_600, MONO_16K, 4);
    let b = |i: usize| &audio[i * 320..(i + 1) * 320];

    // 100 ms = 1600 frames = five 320-frame blocks. Two more are dropped, without waiting.
    for i in 0..5 {
        producer.push(&block(b(i), MONO_16K, i as u64));
    }
    producer.push(&block(b(0), MONO_16K, 5));
    producer.push(&block(b(1), MONO_16K, 6));
    let lost = Overruns {
        blocks: 2,
        frames: 640,
    };
    assert_eq!(producer.overruns(), lost);
    assert_eq!(
        consumer.overruns(),
        lost,
        "visible to the pump before it pops"
    );

    // The pump frees one block; the next push lands and carries the gap in front of it.
    let first = consumer.pop().unwrap();
    assert_eq!(first.block.host_time_ns, 0);
    assert_eq!(first.dropped_blocks_before, 0);
    producer.push(&block(b(2), MONO_16K, 7));

    let rest = drain(&mut consumer);
    let stamps: Vec<u64> = rest.iter().map(|r| r.1).collect();
    assert_eq!(stamps, vec![1, 2, 3, 4, 7]);
    let gap: Vec<(u64, u64)> = rest.iter().map(|r| (r.3, r.4)).collect();
    assert_eq!(gap, vec![(0, 0), (0, 0), (0, 0), (0, 0), (2, 640)]);
    assert_eq!(consumer.overruns(), lost);
}

#[test]
fn a_block_larger_than_the_whole_ring_is_dropped_and_counted() {
    let (mut producer, mut consumer) = capture_ring(MONO_16K, Duration::from_millis(10)).unwrap();
    let audio = signal(1_000, MONO_16K, 5);
    producer.push(&block(&audio, MONO_16K, 0));
    assert!(consumer.pop().is_none());
    assert_eq!(
        consumer.overruns(),
        Overruns {
            blocks: 1,
            frames: 1_000
        }
    );
}

#[test]
fn empty_blocks_are_not_queued() {
    let (mut producer, mut consumer) = capture_ring(MONO_16K, DEFAULT_RING_DURATION).unwrap();
    producer.push(&block(&[], MONO_16K, 0));
    assert!(consumer.pop().is_none());
    assert_eq!(consumer.overruns(), Overruns::default());
}

#[test]
fn ring_wraps_around_without_reordering_or_corruption() {
    // 50 ms ring, 7 ms blocks: indices wrap many times, and blocks straddle the end of the buffer.
    let (mut producer, mut consumer) = capture_ring(MONO_16K, Duration::from_millis(50)).unwrap();
    let audio = signal(112 * 500, MONO_16K, 6);
    for i in 0..500 {
        let samples = &audio[i * 112..(i + 1) * 112];
        producer.push(&block(samples, MONO_16K, i as u64));
        let got = consumer.pop().unwrap();
        assert_eq!(got.block.samples, samples, "block {i}");
        assert_eq!(got.block.host_time_ns, i as u64);
    }
    assert_eq!(consumer.overruns(), Overruns::default());
}

#[test]
fn ring_rejects_an_invalid_format_or_duration() {
    let zero_rate = StreamFormat {
        sample_rate: 0,
        channels: 1,
    };
    let zero_channels = StreamFormat {
        sample_rate: 16_000,
        channels: 0,
    };
    assert_eq!(
        capture_ring(zero_rate, DEFAULT_RING_DURATION).err(),
        Some(RingError::InvalidFormat(zero_rate))
    );
    assert_eq!(
        capture_ring(zero_channels, DEFAULT_RING_DURATION).err(),
        Some(RingError::InvalidFormat(zero_channels))
    );
    assert_eq!(
        capture_ring(MONO_16K, Duration::ZERO).err(),
        Some(RingError::InvalidDuration(Duration::ZERO))
    );
    let hour = Duration::from_secs(3_600);
    assert_eq!(
        capture_ring(MONO_16K, hour).err(),
        Some(RingError::InvalidDuration(hour)),
        "RAM holds seconds, never a session"
    );
}

#[test]
fn ring_carries_blocks_across_threads_in_order_and_accounts_for_every_one() {
    // A realtime-like producer thread races the consumer on a small ring. Whatever the schedule,
    // every block is either delivered in order or counted as dropped, at the right place.
    const BLOCKS: u64 = 5_000;
    let (mut producer, mut consumer) = capture_ring(MONO_16K, Duration::from_millis(40)).unwrap();
    let writer = std::thread::spawn(move || {
        let mut samples = [0f32; 64];
        for i in 0..BLOCKS {
            samples.fill(i as f32);
            producer.push(&AudioBlock {
                samples: &samples,
                format: MONO_16K,
                host_time_ns: i,
            });
        }
    });

    #[derive(Default)]
    struct Tally {
        delivered: u64,
        dropped: u64,
        next: u64,
    }
    fn check(c: &CapturedBlock<'_>, t: &mut Tally) {
        let i = c.block.host_time_ns;
        assert_eq!(
            i,
            t.next + c.dropped_blocks_before,
            "order and gap position"
        );
        assert!(
            c.block.samples.iter().all(|&s| s == i as f32),
            "block {i} intact"
        );
        assert_eq!(c.dropped_frames_before, c.dropped_blocks_before * 64);
        t.dropped += c.dropped_blocks_before;
        t.delivered += 1;
        t.next = i + 1;
    }

    let mut t = Tally::default();
    loop {
        if let Some(c) = consumer.pop() {
            check(&c, &mut t);
        } else if consumer.is_abandoned() {
            break;
        } else {
            std::thread::yield_now();
        }
    }
    // Joining synchronises with the producer's last push; drain whatever raced the check above.
    writer.join().unwrap();
    while let Some(c) = consumer.pop() {
        check(&c, &mut t);
    }
    // Drops after the last delivered block show only in the totals.
    let total = consumer.overruns();
    assert_eq!(t.delivered + total.blocks, BLOCKS);
    assert_eq!(total.blocks - t.dropped, BLOCKS - t.next);
}

#[test]
fn the_ring_is_the_sink_a_capture_source_pushes_into() {
    let mock = Arc::new(MockPlatform::new());
    let platform = mock.platform();
    let mut mic = platform.capture.open_mic(None).unwrap();
    let (producer, mut consumer) = capture_ring(mic.format(), DEFAULT_RING_DURATION).unwrap();
    mic.start(Box::new(producer)).unwrap();

    let audio = signal(960, mic.format(), 7);
    assert!(mock.feed(Channel::Mic, &audio[..480], 10));
    assert!(mock.feed(Channel::Mic, &audio[480..], 20));
    let stats = mic.stop().unwrap();
    assert_eq!(stats.frames, 960);

    let got = drain(&mut consumer);
    assert_eq!(got.len(), 2);
    assert_eq!((got[0].1, got[1].1), (10, 20));
    assert_eq!(got[0].2, mic.format());
    assert_eq!([got[0].0.clone(), got[1].0.clone()].concat(), audio);
    assert!(consumer.is_abandoned(), "stop dropped the sink");
}
