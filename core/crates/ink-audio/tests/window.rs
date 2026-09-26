//! Long audio in windows of at most 60 s, cut at the quietest point, overlapping by 2 s.

use ink_audio::synth::{speech_like, tone};
use ink_audio::window::{Window, WindowConfig, WindowError, Windower, plan_windows, quietest_cut};

const SR: usize = 16_000;
const MAX: u64 = 60 * SR as u64;
const OVERLAP: u64 = 2 * SR as u64;
const SEARCH: u64 = 3 * SR as u64;

/// The invariants every plan keeps: starts at 0, ends at the end, each window at most 60 s,
/// each next window starting exactly 2 s before the previous one ended, every cut in the last
/// 3 s of its window.
fn assert_covers(windows: &[Window], len: usize) {
    if len == 0 {
        assert!(windows.is_empty());
        return;
    }
    assert_eq!(windows[0].start, 0);
    assert_eq!(windows.last().unwrap().end, len as u64);
    for w in windows {
        assert!(w.len() <= MAX, "window {w:?} is {} samples", w.len());
        assert!(w.end > w.start);
    }
    for pair in windows.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        assert_eq!(b.start, a.end - OVERLAP, "{a:?} then {b:?}");
        assert!(
            a.end >= a.start + MAX - SEARCH,
            "cut {} before its search region",
            a.end
        );
    }
}

#[test]
fn cut_lands_in_the_quiet_gap_not_at_the_blind_offset() {
    // Loud everywhere except a 100 ms silence 0.8 s before the end of the search region.
    let mut s = tone(40.0, 440.0, 0.5, 16_000);
    let gap_at = 20 * SR + 12_800;
    s[gap_at..gap_at + 1_600].fill(0.0);
    let cut = quietest_cut(&s, 20 * SR - 24_000..20 * SR + 24_000);
    assert!(
        (gap_at..gap_at + 1_600).contains(&cut),
        "cut {cut} not in the gap {gap_at}..{}",
        gap_at + 1_600
    );
}

#[test]
fn cuts_land_in_the_quietest_region_near_each_boundary() {
    // Speech-level tone with a 100 ms digital-silence gap every 1.37 s: every 3 s search region
    // holds at least two, so every cut must land in one.
    let len = 200 * SR;
    let mut s = tone(200.0, 220.0, 0.3, 16_000);
    let mut gaps = Vec::new();
    let mut at = 0;
    while at + 1_600 < len {
        s[at..at + 1_600].fill(0.0);
        gaps.push(at..at + 1_600);
        at += 21_920; // 1.37 s
    }
    let windows = plan_windows(&s, WindowConfig::default()).unwrap();
    assert_covers(&windows, len);
    assert!(windows.len() >= 4);
    for w in &windows[..windows.len() - 1] {
        let cut = w.end as usize;
        assert!(
            gaps.iter().any(|g| g.contains(&cut)),
            "cut at {:.3} s is not in a quiet gap",
            cut as f64 / SR as f64
        );
    }
}

#[test]
fn windows_cover_every_sample_and_overlap_by_two_seconds() {
    for seconds in [61.0, 119.5, 150.0, 181.3] {
        let s = speech_like(seconds, -30.0, seconds as u64);
        let windows = plan_windows(&s, WindowConfig::default()).unwrap();
        assert_covers(&windows, s.len());
    }
}

#[test]
fn no_quiet_anywhere_still_gives_bounded_windows() {
    // A steady tone has no pause to find: the cut still lands in its region, windows stay under
    // 60 s, and the plan still covers everything with its 2 s overlaps.
    let s = tone(200.0, 1_000.0, 0.5, 16_000);
    let windows = plan_windows(&s, WindowConfig::default()).unwrap();
    assert_covers(&windows, s.len());
    assert_eq!(windows.len(), 4);
}

#[test]
fn digital_silence_everywhere_cuts_as_late_as_it_can() {
    // Every frame is equally quiet; ties go to the latest, so windows are as long as allowed and
    // there are as few seams as possible.
    let s = vec![0.0f32; 150 * SR];
    let windows = plan_windows(&s, WindowConfig::default()).unwrap();
    assert_covers(&windows, s.len());
    for w in &windows[..windows.len() - 1] {
        assert!(w.len() >= MAX - 320, "{w:?}");
    }
}

#[test]
fn audio_no_longer_than_one_window_is_one_window() {
    for len in [1usize, 10 * SR, MAX as usize] {
        let s = vec![0.1f32; len];
        assert_eq!(
            plan_windows(&s, WindowConfig::default()).unwrap(),
            vec![Window {
                start: 0,
                end: len as u64
            }],
            "{len} samples"
        );
    }
    let s = vec![0.1f32; MAX as usize + 1];
    let windows = plan_windows(&s, WindowConfig::default()).unwrap();
    assert_eq!(windows.len(), 2);
    assert_covers(&windows, s.len());
}

#[test]
fn nothing_in_nothing_out() {
    assert!(
        plan_windows(&[], WindowConfig::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn pushing_in_odd_sizes_gives_the_same_windows_and_samples() {
    let s = speech_like(170.0, -30.0, 3);
    let planned = plan_windows(&s, WindowConfig::default()).unwrap();
    let mut w = Windower::new(WindowConfig::default()).unwrap();
    let mut got = Vec::new();
    let sizes = [1usize, 4_097, 16_000, 333, 250_000];
    let (mut at, mut k) = (0, 0);
    while at < s.len() {
        let n = sizes[k % sizes.len()].min(s.len() - at);
        w.push(&s[at..at + n]).unwrap();
        while let Some((window, samples)) = w.next_window() {
            assert_eq!(samples, &s[window.start as usize..window.end as usize]);
            got.push(window);
        }
        at += n;
        k += 1;
    }
    w.finish();
    while let Some((window, samples)) = w.next_window() {
        assert_eq!(samples, &s[window.start as usize..window.end as usize]);
        got.push(window);
    }
    assert_eq!(got, planned);
}

#[test]
fn the_windower_holds_about_one_window_not_the_session() {
    // Ten minutes pushed a second at a time, windows taken as they come: RAM holds seconds and a
    // window, never the session (architecture rule 3).
    let mut w = Windower::new(WindowConfig::default()).unwrap();
    let second = speech_like(1.0, -30.0, 4);
    let mut most = 0;
    let mut windows = 0;
    for _ in 0..600 {
        w.push(&second).unwrap();
        while w.next_window().is_some() {
            windows += 1;
        }
        most = most.max(w.buffered_samples());
    }
    assert!(windows >= 10);
    assert!(
        most <= MAX as usize + SR,
        "held {most} samples ({:.1} s)",
        most as f64 / SR as f64
    );
}

#[test]
fn pushing_after_finish_is_refused() {
    let mut w = Windower::new(WindowConfig::default()).unwrap();
    w.push(&[0.0; 10]).unwrap();
    w.finish();
    assert_eq!(w.push(&[0.0; 10]), Err(WindowError::Finished));
    assert_eq!(
        w.next_window().map(|(win, _)| win),
        Some(Window { start: 0, end: 10 })
    );
    assert!(w.next_window().is_none());
}

#[test]
fn a_config_that_could_not_advance_is_refused() {
    let bad = WindowConfig {
        max_len: 5 * SR,
        overlap: 2 * SR,
        search: 3 * SR,
    };
    assert!(matches!(
        Windower::new(bad),
        Err(WindowError::InvalidConfig(_))
    ));
    let tiny_search = WindowConfig {
        search: 100,
        ..WindowConfig::default()
    };
    assert!(Windower::new(tiny_search).is_err());
}
