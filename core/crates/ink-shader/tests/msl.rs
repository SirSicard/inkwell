//! The Metal Shading Language the Mac app bundles is generated from `shaders/ink.wgsl`; this test
//! fails while it is stale. Regenerate with `cargo run -p ink-shader --bin ink-shader`.

use std::path::Path;

use ink_shader::{INK_WGSL, MSL_OUT, msl_file};

fn repo_root() -> &'static Path {
    // core/crates/ink-shader -> the repository root.
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."))
}

#[test]
fn the_bundled_msl_is_current() {
    let path = repo_root().join(MSL_OUT);
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    let generated = msl_file(INK_WGSL).expect("shaders/ink.wgsl translates");
    // Line endings are normalised: a Windows checkout may turn LF into CRLF.
    assert!(
        committed.replace("\r\n", "\n") == generated,
        "{MSL_OUT} is stale: run `cargo run -p ink-shader --bin ink-shader` in core/"
    );
}

#[test]
fn the_shader_file_is_the_one_built_in() {
    // INK_WGSL is include_str!'d at build time; a stale build would check an old shader.
    let on_disk =
        std::fs::read_to_string(repo_root().join("shaders/ink.wgsl")).expect("shaders/ink.wgsl");
    assert_eq!(
        on_disk.replace("\r\n", "\n"),
        INK_WGSL.replace("\r\n", "\n")
    );
}
