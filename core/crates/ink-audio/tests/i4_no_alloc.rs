//! I4: realtime callbacks are allocation-free.
//!
//! This test binary runs on `assert_no_alloc`'s allocator, which counts every allocation and
//! deallocation made on a thread inside an `assert_no_alloc` scope (warn mode, so a violation fails
//! an assertion with a message instead of aborting the binary). The control tests prove the guard
//! fires, so the I4 tests cannot pass vacuously, for example in a build where the guard is off.

mod common;

use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use assert_no_alloc::{AllocDisabler, assert_no_alloc, violation_count};
use common::{MONO_16K, STEREO_48K, signal};
use ink_audio::{FileReplaySource, Overruns, RealtimeGuard, capture_ring};
use ink_core::mock::MockClock;
use ink_core::{AudioBlock, AudioSink, AudioSource, Channel};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

/// Runs `work` under the no-alloc guard on this thread; returns the (de)allocations it made.
fn allocations_in(work: impl FnOnce()) -> u32 {
    let before = violation_count();
    assert_no_alloc(work);
    violation_count() - before
}

/// A guard for a source's realtime thread: every block's delivery runs under `assert_no_alloc`,
/// and what it catches (and how often it ran) is reported back to the test.
fn counting_guard() -> (RealtimeGuard, Arc<AtomicU64>, Arc<AtomicU64>) {
    let violations = Arc::new(AtomicU64::new(0));
    let calls = Arc::new(AtomicU64::new(0));
    let (v, c) = (violations.clone(), calls.clone());
    let guard: RealtimeGuard = Arc::new(move |work: &mut dyn FnMut()| {
        let before = violation_count();
        assert_no_alloc(work);
        v.fetch_add(u64::from(violation_count() - before), Ordering::Relaxed);
        c.fetch_add(1, Ordering::Relaxed);
    });
    (guard, violations, calls)
}

#[test]
fn control_the_guard_fires_on_an_allocation_and_a_deallocation() {
    assert_eq!(allocations_in(|| {}), 0);
    let caught = allocations_in(|| {
        let v: Vec<u8> = Vec::with_capacity(64);
        black_box(v);
    });
    assert_eq!(caught, 2, "one allocation and one deallocation");
    let kept = std::cell::Cell::new(Vec::<f32>::new());
    assert_eq!(
        allocations_in(|| kept.set(vec![0.0; 16])),
        1,
        "an allocation alone is caught too"
    );
}

#[test]
fn ring_first_push_allocates_nothing() {
    let (producer, mut consumer) = capture_ring(STEREO_48K, Duration::from_secs(2)).unwrap();
    // Pushed through the trait object, as a source calls it.
    let mut sink: Box<dyn AudioSink> = Box::new(producer);
    let audio = signal(512, STEREO_48K, 1);
    let block = AudioBlock {
        samples: &audio,
        format: STEREO_48K,
        host_time_ns: 1,
    };
    assert_eq!(allocations_in(|| sink.push(&block)), 0, "first push");
    for i in 0..500 {
        let block = AudioBlock {
            host_time_ns: i,
            ..block
        };
        assert_eq!(allocations_in(|| sink.push(&block)), 0, "push {i}");
        assert!(consumer.pop().is_some());
    }
}

#[test]
fn ring_overrun_path_allocates_nothing() {
    let (mut producer, mut consumer) = capture_ring(MONO_16K, Duration::from_millis(100)).unwrap();
    let audio = signal(320, MONO_16K, 2);
    let block = AudioBlock {
        samples: &audio,
        format: MONO_16K,
        host_time_ns: 0,
    };
    for _ in 0..5 {
        assert_eq!(allocations_in(|| producer.push(&block)), 0);
    }
    // Full: every further push takes the drop-and-count path.
    for i in 0..20 {
        assert_eq!(allocations_in(|| producer.push(&block)), 0, "overrun {i}");
    }
    let too_big = signal(10_000, MONO_16K, 3);
    let huge = AudioBlock {
        samples: &too_big,
        ..block
    };
    assert_eq!(
        allocations_in(|| producer.push(&huge)),
        0,
        "larger than the ring"
    );
    assert_eq!(
        producer.overruns(),
        Overruns {
            blocks: 21,
            frames: 20 * 320 + 10_000
        }
    );
    // And the push that carries the gap out after the pump frees space.
    assert!(consumer.pop().is_some());
    assert_eq!(
        allocations_in(|| producer.push(&block)),
        0,
        "push after overrun"
    );
}

#[test]
fn replay_delivery_loop_into_the_ring_allocates_nothing() {
    let (guard, violations, calls) = counting_guard();
    let audio = signal(16_000, MONO_16K, 4);
    let (producer, mut consumer) = capture_ring(MONO_16K, Duration::from_secs(2)).unwrap();
    let mut source = FileReplaySource::from_samples(
        audio,
        MONO_16K,
        Channel::Mic,
        Arc::new(MockClock::new(0, 0)),
    )
    .unwrap()
    .with_realtime_guard(guard);
    source.start(Box::new(producer)).unwrap();
    source.wait().unwrap();

    assert_eq!(
        calls.load(Ordering::Relaxed),
        50,
        "the guard wrapped every block"
    );
    assert_eq!(violations.load(Ordering::Relaxed), 0, "and none allocated");
    let mut frames = 0;
    while let Some(c) = consumer.pop() {
        frames += c.block.frames();
    }
    assert_eq!(frames, 16_000);
}

#[test]
fn control_the_replay_guard_catches_a_sink_that_allocates() {
    struct Allocates;
    impl AudioSink for Allocates {
        fn push(&mut self, block: &AudioBlock<'_>) {
            black_box(block.samples.to_vec());
        }
    }
    let (guard, violations, calls) = counting_guard();
    let mut source = FileReplaySource::from_samples(
        signal(3_200, MONO_16K, 5),
        MONO_16K,
        Channel::Mic,
        Arc::new(MockClock::new(0, 0)),
    )
    .unwrap()
    .with_realtime_guard(guard);
    source.start(Box::new(Allocates)).unwrap();
    source.wait().unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 10);
    assert_eq!(
        violations.load(Ordering::Relaxed),
        20,
        "each push's allocation and free, on the replay thread"
    );
}
