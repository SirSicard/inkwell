//! Writes the shells' event types from `schema/events.schema.json`.
//!
//! ```text
//! cargo run -p ink-ffi --bin ink-schema             # regenerate every target
//! cargo run -p ink-ffi --bin ink-schema -- --check  # fail if one is stale
//! ```
//!
//! Targets are paths from the repository root. Swift today; the Windows shell's C# joins in S3.3a.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ink_ffi::schema::{EVENTS_SCHEMA, Schema, swift};

/// Where the generated Swift lives, from the repository root.
const SWIFT_OUT: &str = "mac/Sources/InkBridge/Generated/Events.swift";

fn repo_root() -> PathBuf {
    // core/crates/ink-ffi -> the repository root.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn main() -> ExitCode {
    let check = std::env::args().any(|a| a == "--check");
    let schema = match Schema::parse(EVENTS_SCHEMA) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("schema/events.schema.json: {e}");
            return ExitCode::FAILURE;
        }
    };
    let path = repo_root().join(SWIFT_OUT);
    let generated = swift::swift(&schema);
    if check {
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if current != generated {
            eprintln!("{SWIFT_OUT} is stale: run `cargo run -p ink-ffi --bin ink-schema`");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        eprintln!("{}: {e}", dir.display());
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(&path, generated) {
        eprintln!("{SWIFT_OUT}: {e}");
        return ExitCode::FAILURE;
    }
    println!("wrote {SWIFT_OUT}");
    ExitCode::SUCCESS
}
