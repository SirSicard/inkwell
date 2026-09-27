//! The queue in front of a chain's worker: capture blocks from the pump, and everything else.
//!
//! One FIFO carries both, so a stop or a hotkey event stays in order with the audio around it,
//! as it did on `ink-pipeline`'s single channel. Only audio is bounded: [`push_audio`] never
//! waits, and when [`capacity`](Mailbox::capacity) blocks are already queued the new block is
//! dropped and counted instead. The pump therefore cannot be stalled by a chain that fell behind
//! (an engine call can take seconds), and memory cannot grow without limit while it is behind.
//! Everything else ([`push`]) is small and rare (commands, hotkey edges, a stop) and is never
//! refused while the worker runs.
//!
//! Dropped audio is reported, not only counted: the pump reads [`Pushed::Queued`]'s `after_drop`
//! when the queue takes a block again after dropping some, and turns it into an `audio.dropped`
//! event. [`close`](Mailbox::close) returns an overflow still open at the end.
//!
//! **Threads:** pushes from the pump or a callback thread (a short lock, never a wait on the
//! worker); [`pop`](Mailbox::pop) on the worker only.
//!
//! [`push_audio`]: Mailbox::push_audio
//! [`push`]: Mailbox::push

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// Blocks a chain's queue holds: at 10 ms blocks, 20 s of audio for two sides. A chain that is
/// this far behind has stalled, and dropping (and saying so) is better than growing.
pub const DEFAULT_AUDIO_CAPACITY: usize = 4_000;

/// Audio the pump dropped because the queue was full, in one stretch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Overflow {
    /// Blocks dropped.
    pub blocks: u64,
    /// Samples in them.
    pub samples: u64,
}

/// What became of a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pushed {
    /// Queued. `after_drop` holds the stretch dropped just before it, if any: report it.
    Queued {
        /// The overflow this block ended.
        after_drop: Option<Overflow>,
    },
    /// The queue was full: dropped and counted.
    Dropped,
    /// The worker has stopped: nothing will take it.
    Closed,
}

/// What [`Mailbox::pop`] found.
pub enum Pop<A, C> {
    /// The next item.
    Item(Item<A, C>),
    /// The deadline passed first.
    TimedOut,
    /// The mailbox is closed and empty.
    Closed,
}

/// Something queued for the worker: audio (bounded) or anything else (not).
pub enum Item<A, C> {
    /// A capture block and its sample count.
    Audio(A, usize),
    /// Anything else.
    Other(C),
}

struct State<A, C> {
    items: VecDeque<Item<A, C>>,
    audio: usize,
    open: Overflow,
    total: Overflow,
    closed: bool,
}

/// See the module docs. `A` is a capture block, `C` anything else.
pub struct Mailbox<A, C> {
    state: Mutex<State<A, C>>,
    ready: Condvar,
    capacity: usize,
}

impl<A, C> Mailbox<A, C> {
    /// A mailbox holding at most `capacity` audio blocks (at least one).
    pub fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::new(State {
                items: VecDeque::new(),
                audio: 0,
                open: Overflow::default(),
                total: Overflow::default(),
                closed: false,
            }),
            ready: Condvar::new(),
            capacity: capacity.max(1),
        }
    }

    /// The most audio blocks queued at once.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    // Poison is ignored: every critical section leaves the queue consistent, and a panicking
    // worker must not also wedge the pump.
    fn lock(&self) -> MutexGuard<'_, State<A, C>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// **Pump.** Queues a block of `samples` samples, or drops it when the queue is full. Never
    /// waits for the worker.
    pub fn push_audio(&self, block: A, samples: usize) -> Pushed {
        let mut s = self.lock();
        if s.closed {
            return Pushed::Closed;
        }
        if s.audio >= self.capacity {
            s.open.blocks += 1;
            s.open.samples += samples as u64;
            s.total.blocks += 1;
            s.total.samples += samples as u64;
            return Pushed::Dropped;
        }
        s.items.push_back(Item::Audio(block, samples));
        s.audio += 1;
        let after_drop = (s.open.blocks > 0).then(|| std::mem::take(&mut s.open));
        drop(s);
        self.ready.notify_one();
        Pushed::Queued { after_drop }
    }

    /// **Any thread but realtime.** Queues anything but audio. Returns it back when the worker has
    /// stopped.
    pub fn push(&self, item: C) -> Result<(), C> {
        let mut s = self.lock();
        if s.closed {
            return Err(item);
        }
        s.items.push_back(Item::Other(item));
        drop(s);
        self.ready.notify_one();
        Ok(())
    }

    /// **Worker.** The next item, waiting for one until `deadline` (forever with `None`).
    pub fn pop(&self, deadline: Option<Instant>) -> Pop<A, C> {
        let mut s = self.lock();
        loop {
            if let Some(item) = s.items.pop_front() {
                if matches!(item, Item::Audio(..)) {
                    s.audio -= 1;
                }
                return Pop::Item(item);
            }
            if s.closed {
                return Pop::Closed;
            }
            s = match deadline {
                None => self.ready.wait(s).unwrap_or_else(PoisonError::into_inner),
                Some(at) => {
                    let wait = at.saturating_duration_since(Instant::now());
                    if wait == Duration::ZERO {
                        return Pop::TimedOut;
                    }
                    self.ready
                        .wait_timeout(s, wait)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
            };
        }
    }

    /// Refuses everything from now on and wakes the worker. What is queued can still be popped.
    /// Returns an overflow that no later block reported.
    pub fn close(&self) -> Option<Overflow> {
        let mut s = self.lock();
        s.closed = true;
        let open = (s.open.blocks > 0).then(|| std::mem::take(&mut s.open));
        drop(s);
        self.ready.notify_all();
        open
    }

    /// Everything dropped so far.
    pub fn dropped(&self) -> Overflow {
        self.lock().total
    }

    /// Audio blocks queued now.
    pub fn queued_audio(&self) -> usize {
        self.lock().audio
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio(item: Pop<u32, &'static str>) -> u32 {
        match item {
            Pop::Item(Item::Audio(a, _)) => a,
            _ => panic!("expected audio"),
        }
    }

    #[test]
    fn full_queue_drops_new_blocks_and_reports_them_on_the_next_one() {
        let m: Mailbox<u32, &str> = Mailbox::new(2);
        assert_eq!(m.push_audio(1, 160), Pushed::Queued { after_drop: None });
        assert_eq!(m.push_audio(2, 160), Pushed::Queued { after_drop: None });
        assert_eq!(m.push_audio(3, 160), Pushed::Dropped);
        assert_eq!(m.push_audio(4, 100), Pushed::Dropped);
        // Other items are not bounded by the audio.
        m.push("stop").unwrap();
        assert_eq!(audio(m.pop(None)), 1);
        assert_eq!(
            m.push_audio(5, 160),
            Pushed::Queued {
                after_drop: Some(Overflow {
                    blocks: 2,
                    samples: 260
                })
            }
        );
        assert_eq!(m.push_audio(6, 160), Pushed::Dropped);
        assert_eq!(
            m.dropped(),
            Overflow {
                blocks: 3,
                samples: 420
            }
        );
        assert_eq!(
            m.close(),
            Some(Overflow {
                blocks: 1,
                samples: 160
            })
        );
    }

    #[test]
    fn order_is_kept_across_audio_and_other_items() {
        let m: Mailbox<u32, &str> = Mailbox::new(8);
        m.push_audio(1, 1);
        m.push("press").unwrap();
        m.push_audio(2, 1);
        assert_eq!(audio(m.pop(None)), 1);
        assert!(matches!(m.pop(None), Pop::Item(Item::Other("press"))));
        assert_eq!(audio(m.pop(None)), 2);
    }

    #[test]
    fn a_pop_with_a_deadline_returns_none_when_it_passes() {
        let m: Mailbox<u32, &str> = Mailbox::new(1);
        let t = Instant::now();
        assert!(matches!(
            m.pop(Some(t + Duration::from_millis(20))),
            Pop::TimedOut
        ));
        assert!(t.elapsed() >= Duration::from_millis(20));
    }

    #[test]
    fn closing_refuses_pushes_and_drains_what_is_queued() {
        let m: Mailbox<u32, &str> = Mailbox::new(4);
        m.push_audio(7, 1);
        assert_eq!(m.close(), None);
        assert_eq!(m.push_audio(8, 1), Pushed::Closed);
        assert_eq!(m.push("x"), Err("x"));
        assert_eq!(audio(m.pop(None)), 7);
        assert!(matches!(m.pop(None), Pop::Closed));
    }
}
