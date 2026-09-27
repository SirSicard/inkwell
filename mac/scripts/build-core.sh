#!/usr/bin/env bash
# Builds the Rust core (core/crates/ink-ffi) as a static library and wraps it, with inkwell.h and
# its module map, into mac/build/InkCore.xcframework: the InkCore binary target in Package.swift.
#
#   mac/scripts/build-core.sh                      release, engine features off (what CI builds)
#   INK_CORE_PROFILE=debug mac/scripts/build-core.sh
#   INK_CORE_FEATURES=engine-llama mac/scripts/build-core.sh
#
# Extra cargo flags go in CARGO_FLAGS (for example --offline). Apple Silicon only: the one Rust
# target built here is the host's. An Intel slice needs `rustup target add x86_64-apple-darwin`,
# a second `cargo build --target x86_64-apple-darwin`, and `lipo -create` of the two archives
# before -create-xcframework (a decision for the release pipeline).
set -euo pipefail

mac="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$mac/.." && pwd)"
profile="${INK_CORE_PROFILE:-release}"
features="${INK_CORE_FEATURES:-}"
# Package.swift's platform. Without it the C parts of the core (SQLite) are compiled for the
# building Mac's own macOS version, and the linker warns when the app targets an older one.
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-26.0}"

args=(build -p ink-ffi --lib --locked)
if [ "$profile" = "release" ]; then
  args+=(--release)
fi
if [ -n "$features" ]; then
  args+=(--features "$features")
fi
# shellcheck disable=SC2206 # CARGO_FLAGS is split on purpose
args+=(${CARGO_FLAGS:-})
(cd "$root/core" && cargo "${args[@]}")

lib="$root/core/target/$profile/libink_ffi.a"
headers="$mac/build/headers"
out="$mac/build/InkCore.xcframework"
rm -rf "$headers" "$out"
mkdir -p "$headers"
cp "$root/core/crates/ink-ffi/include/inkwell.h" "$root/core/crates/ink-ffi/include/module.modulemap" "$headers/"
xcodebuild -create-xcframework -library "$lib" -headers "$headers" -output "$out" >/dev/null
echo "built $out ($profile${features:+, features: $features})"
