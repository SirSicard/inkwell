//! The Swift event types are generated from `schema/events.schema.json` and checked in; this
//! fails while they are stale. Regenerate with `cargo run -p ink-ffi --bin ink-schema`.

use std::path::Path;

use ink_ffi::schema::{EVENTS_SCHEMA, Schema, swift};

#[test]
fn the_generated_swift_is_current() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../mac/Sources/InkBridge/Generated/Events.swift");
    // A Windows checkout may turn line ends into CRLF; the content is what must match.
    let checked_in = std::fs::read_to_string(&path)
        .expect("the generated Swift is checked in")
        .replace("\r\n", "\n");
    let generated = swift::swift(&Schema::parse(EVENTS_SCHEMA).unwrap());
    assert!(
        checked_in == generated,
        "mac/Sources/InkBridge/Generated/Events.swift is stale: run \
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
