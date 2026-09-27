//! The event thread: the one core thread every event reaches the shell on, in order.
//!
//! Producers (workers, the pump, the command thread) hand events over by value and never wait:
//! the queue is unbounded because events are small and the shell's callback only hops to its
//! main thread. A shell whose callback blocks makes it grow; the header forbids that.
//!
//! [`Hub::stop`] delivers everything queued before it and then ends the thread, so once it
//! returns the shell's callback is never called again.

use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use serde_json::Value;

/// What receives each event's JSON, on the event thread.
pub type EventOut = Box<dyn FnMut(&str) + Send>;

enum Msg {
    Event(Value),
    Stop,
}

/// A handle for emitting events. Cheap to clone; any thread but realtime.
#[derive(Clone)]
pub struct Events {
    tx: Sender<Msg>,
}

impl Events {
    /// Queues `event` for the shell. After the hub stopped it is dropped: by then every producer
    /// has been joined, and nothing is waiting for it.
    pub fn emit(&self, event: Value) {
        let _ = self.tx.send(Msg::Event(event));
    }
}

/// The running event thread.
pub struct Hub {
    events: Events,
    thread: JoinHandle<()>,
}

impl Hub {
    /// Starts the thread; `out` receives every event.
    pub fn start(out: EventOut) -> io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("ink-events".into())
            .spawn(move || deliver(&rx, out))?;
        Ok(Self {
            events: Events { tx },
            thread,
        })
    }

    /// A handle for emitting.
    pub fn events(&self) -> Events {
        self.events.clone()
    }

    /// Delivers everything queued so far, then ends the thread.
    pub fn stop(self) {
        let _ = self.events.tx.send(Msg::Stop);
        if self.thread.join().is_err() {
            log::error!("the event thread panicked outside its per-event boundary");
        }
    }
}

fn deliver(rx: &Receiver<Msg>, mut out: EventOut) {
    while let Ok(Msg::Event(event)) = rx.recv() {
        let json = event.to_string();
        // A callback that panics (a Rust test's) must not end delivery for everything after it.
        // The payload is not logged: an event can hold the user's words (I5).
        if panic::catch_unwind(AssertUnwindSafe(|| out(&json))).is_err() {
            log::error!("the event callback panicked; the event was lost");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::*;

    #[test]
    fn events_arrive_in_order_on_one_named_thread_and_stop_flushes_them() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let hub = Hub::start(Box::new(move |e| {
            let name = thread::current().name().map(str::to_owned);
            sink.lock().unwrap().push((name, e.to_owned()));
        }))
        .unwrap();
        let events = hub.events();
        let producers: Vec<_> = (0..4)
            .map(|p| {
                let events = events.clone();
                thread::spawn(move || {
                    for n in 0..50 {
                        events.emit(json!({"type": "t", "p": p, "n": n}));
                    }
                })
            })
            .collect();
        for p in producers {
            p.join().unwrap();
        }
        hub.stop();
        events.emit(json!({"type": "late"}));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 200);
        assert!(seen.iter().all(|(n, _)| n.as_deref() == Some("ink-events")));
        for p in 0..4 {
            let ns: Vec<i64> = seen
                .iter()
                .map(|(_, e)| serde_json::from_str::<Value>(e).unwrap())
                .filter(|v| v["p"] == p)
                .map(|v| v["n"].as_i64().unwrap())
                .collect();
            assert_eq!(ns, (0..50).collect::<Vec<_>>(), "producer {p} in order");
        }
    }

    #[test]
    fn a_panicking_callback_loses_one_event_not_the_rest() {
        let seen = Arc::new(Mutex::new(0));
        let sink = seen.clone();
        let hub = Hub::start(Box::new(move |e| {
            if e.contains("boom") {
                panic!("test callback");
            }
            *sink.lock().unwrap() += 1;
        }))
        .unwrap();
        let events = hub.events();
        events.emit(json!({"type": "boom"}));
        events.emit(json!({"type": "ok"}));
        hub.stop();
        assert_eq!(*seen.lock().unwrap(), 1);
    }
}
