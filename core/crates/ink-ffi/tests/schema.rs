//! The shells' event types are generated from `schema/events.schema.json` and checked in; these
//! fail while they are stale. Regenerate with `cargo run -p ink-ffi --bin ink-schema`.

use std::path::Path;

use ink_ffi::schema::{EVENTS_SCHEMA, Schema, csharp, swift};

/// A generated file as checked in. A Windows checkout may turn line ends into CRLF; the content is
/// what must match.
fn checked_in(from_repo_root: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join(from_repo_root);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{from_repo_root} is checked in: {e}"))
        .replace("\r\n", "\n")
}

#[test]
fn the_generated_swift_is_current() {
    let generated = swift::swift(&Schema::parse(EVENTS_SCHEMA).unwrap());
    assert!(
        checked_in("mac/Sources/InkBridge/Generated/Events.swift") == generated,
        "mac/Sources/InkBridge/Generated/Events.swift is stale: run \
         `cargo run -p ink-ffi --bin ink-schema`"
    );
}

#[test]
fn the_generated_csharp_is_current() {
    let generated = csharp::csharp(&Schema::parse(EVENTS_SCHEMA).unwrap());
    assert!(
        checked_in("windows/Inkwell.Core/Generated/Events.g.cs") == generated,
        "windows/Inkwell.Core/Generated/Events.g.cs is stale: run \
         `cargo run -p ink-ffi --bin ink-schema`"
    );
}

#[test]
fn the_schema_file_is_the_one_built_in() {
    // The core validates against the copy compiled in; a schema edit rebuilds it.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../schema/events.schema.json");
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        EVENTS_SCHEMA,
        "rebuild the crate"
    );
}
