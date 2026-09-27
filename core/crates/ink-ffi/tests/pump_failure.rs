//! A pump that panics still ends its meeting. Only the pump tells the worker that capture ended,
//! so a pump that died silently would leave the worker ticking forever, and shutdown waiting for
//! it. The pump runs behind a panic boundary: capture stops, the shell hears it, the worker is
//! told to stop, and shutdown returns.

mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc;
use std::time::Duration;

use common::*;
use ink_core::{AudioSink, AudioSource, Channel, PlatformError, SourceStats, StreamFormat};
use ink_ffi::meeting::CaptureSide;

/// A capture source whose start panics: it runs on the pump's thread.
struct PanicsOnStart;

impl AudioSource for PanicsOnStart {
    fn channel(&self) -> Channel {
        Channel::Mic
    }

    fn format(&self) -> StreamFormat {
        StreamFormat::CANONICAL
    }

    fn start(&mut self, _: Box<dyn AudioSink>) -> Result<(), PlatformError> {
        panic!("scripted capture fault");
    }

    fn stop(&mut self) -> Result<SourceStats, PlatformError> {
        Ok(SourceStats::default())
    }
}

#[test]
fn a_pump_that_panics_ends_the_meeting_and_shutdown_still_returns() {
    let dir = TempDir::new("pump-panic");
    let loader = MockLoader::new(Behaviour::Say("x".into()));
    let installer = Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: None,
        installs: AtomicUsize::new(0),
    });
    let (core, events) = start(&dir, &[test_row(ROW_ID)], loader, installer);
    core.start_meeting(
        vec![CaptureSide {
            source: Box::new(PanicsOnStart),
            ring: Duration::from_secs(2),
            start_at: None,
        }],
        Some("Faulty capture".into()),
    )
    .unwrap();

    let failed = events.wait_type("meeting.capture_failed", Duration::from_secs(10));
    let started = events.wait_type("meeting.started", Duration::ZERO);
    assert_eq!(failed["record"], started["record"]);
    // The worker was told capture ended: it stopped the chain and ran the final pass.
    events.wait_type("meeting.stopped", Duration::from_secs(10));
    events.wait_type("meeting.finished", Duration::from_secs(10));

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        core.shutdown();
        let _ = tx.send(());
    });
    assert!(
        rx.recv_timeout(Duration::from_secs(10)).is_ok(),
        "shutdown hung on the meeting"
    );
    events.assert_valid();
}
