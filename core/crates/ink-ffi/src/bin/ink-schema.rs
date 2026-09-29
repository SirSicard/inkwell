//! Writes the shells' event types from `schema/events.schema.json`.
//!
//! ```text
//! cargo run -p ink-ffi --bin ink-schema             # regenerate every target
//! cargo run -p ink-ffi --bin ink-schema -- --check  # fail if one is stale
//! ```
//!
//! Targets are paths from the repository root: the Mac shell's Swift and the Windows shell's C#.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ink_ffi::schema::{EVENTS_SCHEMA, Schema, csharp, swift};

/// Where the generated Swift lives, from the repository root.
const SWIFT_OUT: &str = "mac/Sources/InkBridge/Generated/Events.swift";
/// Where the generated C# lives, from the repository root.
const CSHARP_OUT: &str = "windows/Inkwell.Core/Generated/Events.g.cs";

fn repo_root() -> PathBuf {
    // core/crates/ink-ffi -> the repository root.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// Checks or writes one target; false when it failed or is stale.
fn target(out: &str, generated: &str, check: bool) -> bool {
    let path = repo_root().join(out);
    if check {
        // A Windows checkout may turn line ends into CRLF; the content is what must match.
        let current = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        if current != generated {
            eprintln!("{out} is stale: run `cargo run -p ink-ffi --bin ink-schema`");
            return false;
        }
        return true;
    }
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        eprintln!("{}: {e}", dir.display());
        return false;
    }
    if let Err(e) = std::fs::write(&path, generated) {
        eprintln!("{out}: {e}");
        return false;
    }
    println!("wrote {out}");
    true
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
    // Both run, so one stale target does not hide the other.
    let swift_ok = target(SWIFT_OUT, &swift::swift(&schema), check);
    let csharp_ok = target(CSHARP_OUT, &csharp::csharp(&schema), check);
    if swift_ok && csharp_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
