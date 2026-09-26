//! A sliding window over one stream: absolute sample indices, fixed capacity, contiguous reads.
//!
//! Allocated once. Appending compacts in place when it reaches the end, so pushes allocate
//! nothing, and a stream that runs further ahead than the capacity is refused rather than
//! buffered without bound: RAM holds seconds, never a session.

/// See the module docs.
pub(crate) struct Sliding {
    data: Vec<f32>,
    start: usize,
    len: usize,
    /// Absolute index of `data[start]`.
    base: u64,
}

/// The stream would outgrow the buffer.
#[derive(Debug)]
pub(crate) struct Full;

impl Sliding {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            data: vec![0.0; capacity],
            start: 0,
            len: 0,
            base: 0,
        }
    }

    /// Absolute index of the first held sample.
    pub(crate) fn base(&self) -> u64 {
        self.base
    }

    /// Absolute index one past the last held sample: how much of the stream has arrived.
    pub(crate) fn end(&self) -> u64 {
        self.base + self.len as u64
    }

    /// The held samples, `base()..end()`.
    pub(crate) fn as_slice(&self) -> &[f32] {
        &self.data[self.start..self.start + self.len]
    }

    /// Samples that can still be pushed.
    pub(crate) fn room(&self) -> usize {
        self.data.len() - self.len
    }

    pub(crate) fn push(&mut self, samples: &[f32]) -> Result<(), Full> {
        if self.len + samples.len() > self.data.len() {
            return Err(Full);
        }
        if self.start + self.len + samples.len() > self.data.len() {
            self.data.copy_within(self.start..self.start + self.len, 0);
            self.start = 0;
        }
        let at = self.start + self.len;
        self.data[at..at + samples.len()].copy_from_slice(samples);
        self.len += samples.len();
        Ok(())
    }

    /// Forgets every sample before absolute index `index` (no-op for indices already gone).
    pub(crate) fn discard_before(&mut self, index: u64) {
        if index <= self.base {
            return;
        }
        let n = ((index - self.base) as usize).min(self.len);
        self.start += n;
        self.len -= n;
        self.base += n as u64;
        if self.len == 0 {
            self.start = 0;
        }
    }

    /// Copies absolute `from..from + out.len()` into `out`, with zeros where the stream has no
    /// samples (before its start, after its end, or already discarded).
    pub(crate) fn copy_out(&self, from: u64, out: &mut [f32]) {
        for (i, o) in out.iter_mut().enumerate() {
            let idx = from + i as u64;
            *o = if idx >= self.base && idx < self.end() {
                self.data[self.start + (idx - self.base) as usize]
            } else {
                0.0
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_keeps_absolute_indices_across_compaction() {
        let mut b = Sliding::with_capacity(8);
        b.push(&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0]).unwrap();
        b.discard_before(4);
        assert_eq!((b.base(), b.end()), (4, 6));
        b.push(&[6.0, 7.0, 8.0, 9.0, 10.0]).unwrap();
        assert_eq!(b.as_slice(), &[4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]);
        let mut out = [9.9f32; 4];
        b.copy_out(9, &mut out);
        assert_eq!(out, [9.0, 10.0, 0.0, 0.0]);
        b.copy_out(2, &mut out);
        assert_eq!(out, [0.0, 0.0, 4.0, 5.0]);
    }

    #[test]
    fn a_push_past_the_capacity_is_refused_and_changes_nothing() {
        let mut b = Sliding::with_capacity(4);
        b.push(&[1.0, 2.0, 3.0]).unwrap();
        assert!(b.push(&[4.0, 5.0]).is_err());
        assert_eq!(b.as_slice(), &[1.0, 2.0, 3.0]);
        b.discard_before(100);
        assert_eq!((b.base(), b.end()), (3, 3));
    }
}
