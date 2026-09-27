//! The command thread outlives a command that panics: the shell hears which command failed, and
//! the next command runs.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::*;

#[test]
fn a_command_that_panics_is_reported_and_the_next_one_still_runs() {
    let dir = TempDir::new("command-panic");
    let loader = MockLoader::new(Behaviour::Say("x".into()));
    let installer = Arc::new(MockInstaller {
        generation: loader.generation.clone(),
        gate: None,
        installs: AtomicUsize::new(0),
    });
    let (core, events) = start(&dir, &[test_row(ROW_ID)], loader.clone(), installer);

    loader.panic_on_load.store(true, Ordering::SeqCst);
    core.command(r#"{"cmd":"model.warm","job":"dictation_final","id":"w1"}"#)
        .unwrap();
    let failed = events.wait_type("command.failed", Duration::from_secs(5));
    assert_eq!(failed["command"], "model.warm");
    assert_eq!(failed["id"], "w1");

    // Nothing is left half done: residency cleared the load it had started, and the command
    // thread serves the next command.
    loader.panic_on_load.store(false, Ordering::SeqCst);
    core.command(r#"{"cmd":"model.warm","job":"dictation_final"}"#)
        .unwrap();
    let warmed = events.wait_type("model.warmed", Duration::from_secs(5));
    assert_eq!(warmed["id"], ROW_ID);
    // And one after that, a model update, whose gate hold is taken and released as usual.
    core.command(&format!(
        r#"{{"cmd":"model.update","model":"{ROW_ID}","next":"{ROW_ID}"}}"#
    ))
    .unwrap();
    let updated = events.wait_type("model.update_finished", Duration::from_secs(5));
    assert_eq!(updated["ok"], true, "{updated}");
    core.shutdown();
    events.assert_valid();
}
