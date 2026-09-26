//! Bands: three energy bands that drive the ink renderer, and the copy-out reader the shell uses.
//!
//! [`BandAnalyzer`] runs one 512-point FFT (Hann window, 32 ms) every 256 samples (16 ms, 62.5 per
//! second, above the renderer's 60 fps) of 16 kHz audio and sums its bins into three bands:
//!
//! | Band | Edges | Bins (31.25 Hz each) | What it carries |
//! |---|---|---|---|
//! | low | [`LOW_HZ`], 80–500 Hz | 3–15 | voice fundamental and first formant |
//! | mid | [`MID_HZ`], 500 Hz–2 kHz | 16–63 | the formants that carry vowels |
//! | high | [`HIGH_HZ`], 2–8 kHz | 64–255 | fricatives and sibilants |
//!
//! DC, the rumble below 80 Hz and the Nyquist bin belong to no band. Each value is the RMS
//! amplitude of the band's content in linear full-scale units (a 0 dBFS sine inside a band reads
//! 0.707), so silence reads exactly zero. What the ink does with these numbers (a dB scale,
//! smoothing) is the renderer's business. The analyzer measures what it is fed: feed it the
//! gained stream, or quiet speech draws almost nothing.
//!
//! # The copy-out reader
//!
//! [`bands_channel`] makes one [`BandsWriter`] (the pump) and any number of [`BandsReader`]s (the
//! shell's render thread, through the C ABI). The writer publishes into a sequence lock built from
//! atomics: it never waits, and a reader copies the latest values out, retrying in the rare case
//! that it overlapped a publish. No raw shared pointers, and no lock a renderer could block on or
//! a reader could hold against the pump. Each read also returns how many publishes there have
//! been, so the renderer can tell a still frame from a new one (and draw nothing when idle).
//!
//! # Threading and allocation
//!
//! [`BandAnalyzer::new`] allocates (the FFT plan and its buffers). [`BandAnalyzer::process`] and
//! [`BandsWriter::publish`] run on the pump and never allocate. [`BandsReader::read`] runs on any
//! thread, never allocates and never blocks the writer.

use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};

use ink_core::CANONICAL_RATE;
use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

/// FFT length: 32 ms at 16 kHz.
pub const FFT_LEN: usize = 512;

/// Samples between FFTs: 16 ms at 16 kHz.
pub const HOP: usize = 256;

/// The low band's edges in Hz (start inclusive, end exclusive).
pub const LOW_HZ: Range<f32> = 80.0..500.0;

/// The mid band's edges in Hz.
pub const MID_HZ: Range<f32> = 500.0..2_000.0;

/// The high band's edges in Hz.
pub const HIGH_HZ: Range<f32> = 2_000.0..8_000.0;

/// One hop's bands: RMS amplitude per band, linear full scale.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bands {
    /// 80–500 Hz.
    pub low: f32,
    /// 500 Hz–2 kHz.
    pub mid: f32,
    /// 2–8 kHz.
    pub high: f32,
}

/// Runs one FFT per hop over 16 kHz mono audio. See the module docs.
pub struct BandAnalyzer {
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    /// The last `FFT_LEN` samples, oldest at `next`.
    history: Vec<f32>,
    next: usize,
    since_hop: usize,
    frame: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    /// Converts a band's summed |X|² into its mean square: 2 / (N · Σw²).
    norm: f32,
    bins: [Range<usize>; 3],
    hops: u64,
}

impl Default for BandAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl BandAnalyzer {
    /// An analyzer with an empty (silent) history. **Allocates.**
    pub fn new() -> Self {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(FFT_LEN);
        // Periodic Hann, the usual window for overlapping frames.
        let window: Vec<f32> = (0..FFT_LEN)
            .map(|n| {
                let x = std::f64::consts::TAU * n as f64 / FFT_LEN as f64;
                (0.5 - 0.5 * x.cos()) as f32
            })
            .collect();
        let window_power: f32 = window.iter().map(|w| w * w).sum();
        let bin = |hz: f32| (hz * FFT_LEN as f32 / CANONICAL_RATE as f32).ceil() as usize;
        let band = |r: &Range<f32>| bin(r.start)..bin(r.end);
        Self {
            frame: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            fft,
            window,
            history: vec![0.0; FFT_LEN],
            next: 0,
            since_hop: 0,
            norm: 2.0 / (FFT_LEN as f32 * window_power),
            bins: [band(&LOW_HZ), band(&MID_HZ), band(&HIGH_HZ)],
            hops: 0,
        }
    }

    /// FFTs run so far (one per [`HOP`] samples pushed).
    pub fn hops(&self) -> u64 {
        self.hops
    }

    /// Takes the next audio, runs one FFT for every hop it completes, and returns the last hop's
    /// bands, or `None` if no hop completed. **Pump.** Never allocates.
    pub fn process(&mut self, audio: &[f32]) -> Option<Bands> {
        let mut latest = None;
        for &s in audio {
            self.history[self.next] = s;
            self.next = (self.next + 1) % FFT_LEN;
            self.since_hop += 1;
            if self.since_hop == HOP {
                self.since_hop = 0;
                latest = Some(self.analyse());
            }
        }
        latest
    }

    fn analyse(&mut self) -> Bands {
        // Oldest first: history[next..] then history[..next].
        let (newer, older) = self.history.split_at(self.next);
        let ordered = older.iter().chain(newer);
        for ((f, &s), &w) in self.frame.iter_mut().zip(ordered).zip(&self.window) {
            *f = s * w;
        }
        // Cannot fail: the three buffers came from this plan's own make_* calls, so their lengths
        // are the ones it checks for.
        self.fft
            .process_with_scratch(&mut self.frame, &mut self.spectrum, &mut self.scratch)
            .expect("buffers sized by the FFT plan itself");
        self.hops += 1;
        let power = |bins: &Range<usize>| -> f32 {
            let sum: f32 = self.spectrum[bins.clone()]
                .iter()
                .map(|c| c.norm_sqr())
                .sum();
            (sum * self.norm).sqrt()
        };
        Bands {
            low: power(&self.bins[0]),
            mid: power(&self.bins[1]),
            high: power(&self.bins[2]),
        }
    }
}

/// A copy of the latest published bands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandsSnapshot {
    /// The bands (all zero before the first publish).
    pub bands: Bands,
    /// Publishes so far. Unchanged since the last read means nothing new to draw.
    pub published: u64,
}

/// The single writer: the pump. Not `Clone`, and `publish` takes `&mut self`, so there is only
/// ever one writer, which the sequence lock relies on.
pub struct BandsWriter(Arc<Cell>);

/// A reader for any thread. Cheap to clone.
#[derive(Clone)]
pub struct BandsReader(Arc<Cell>);

/// The sequence lock. `seq` is odd while a publish is in progress; `seq / 2` is the number of
/// completed publishes. The values are `f32` bits in atomics, so no read is ever undefined, and
/// the sequence check discards any read that overlapped a publish.
#[derive(Default)]
struct Cell {
    seq: AtomicU64,
    low: AtomicU32,
    mid: AtomicU32,
    high: AtomicU32,
}

/// One writer and a reader of the same bands. Clone the reader for more threads.
pub fn bands_channel() -> (BandsWriter, BandsReader) {
    let cell = Arc::new(Cell::default());
    (BandsWriter(cell.clone()), BandsReader(cell))
}

impl BandsWriter {
    /// Publishes `bands` as the latest. **Pump.** Never waits and never allocates.
    pub fn publish(&mut self, bands: Bands) {
        let cell = &*self.0;
        // Only this writer stores to `seq`, so this load sees its own last store.
        let seq = cell.seq.load(Ordering::Relaxed);
        cell.seq.store(seq.wrapping_add(1), Ordering::Relaxed);
        // Orders the odd sequence before the value stores: a reader that sees any new value also
        // sees the odd (or a later) sequence on its second load, and retries.
        fence(Ordering::Release);
        cell.low.store(bands.low.to_bits(), Ordering::Relaxed);
        cell.mid.store(bands.mid.to_bits(), Ordering::Relaxed);
        cell.high.store(bands.high.to_bits(), Ordering::Relaxed);
        cell.seq.store(seq.wrapping_add(2), Ordering::Release);
    }
}

impl BandsReader {
    /// Copies the latest published bands out. **Any thread.** Lock-free: it retries only while a
    /// publish (three stores) is in progress, and never blocks the writer.
    pub fn read(&self) -> BandsSnapshot {
        let cell = &*self.0;
        loop {
            let before = cell.seq.load(Ordering::Acquire);
            if before & 1 == 0 {
                let bands = Bands {
                    low: f32::from_bits(cell.low.load(Ordering::Relaxed)),
                    mid: f32::from_bits(cell.mid.load(Ordering::Relaxed)),
                    high: f32::from_bits(cell.high.load(Ordering::Relaxed)),
                };
                // Pairs with the writer's release fence (see `publish`).
                fence(Ordering::Acquire);
                if cell.seq.load(Ordering::Relaxed) == before {
                    return BandsSnapshot {
                        bands,
                        published: before / 2,
                    };
                }
            }
            std::hint::spin_loop();
        }
    }
}
