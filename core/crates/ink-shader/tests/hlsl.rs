//! The HLSL the Windows shell embeds is generated from `shaders/ink.wgsl`; this test fails while it
//! is stale. Regenerate with `cargo run -p ink-shader --bin ink-shader`.

use std::path::Path;

use ink_shader::{HLSL_OUT, INK_WGSL, hlsl_file};

fn repo_root() -> &'static Path {
    // core/crates/ink-shader -> the repository root.
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."))
}

#[test]
fn the_embedded_hlsl_is_current() {
    let path = repo_root().join(HLSL_OUT);
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    let generated = hlsl_file(INK_WGSL).expect("shaders/ink.wgsl translates");
    // Line endings are normalised: a Windows checkout may turn LF into CRLF.
    assert!(
        committed.replace("\r\n", "\n") == generated,
        "{HLSL_OUT} is stale: run `cargo run -p ink-shader --bin ink-shader` in core/"
    );
}
