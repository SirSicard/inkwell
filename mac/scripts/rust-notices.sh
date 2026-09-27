#!/usr/bin/env bash
# The licence notices of the Rust crates the release links into the core, which Settings > About
# shows (mac/Sources/Inkwell/Generated/RustNotices.swift): regenerates them, or with --check fails
# when the checked-in file differs from a fresh run, so a dependency change cannot ship without its
# notice. The Mac CI job runs --check, and so does release day (docs/RELEASING.md).
#
#   mac/scripts/rust-notices.sh [--check]
#
# The generator (core/crates/ink-ffi/src/bin/ink-notices) is offline: it reads cargo's resolution
# of the release build and each crate's unpacked package in the local registry. So this first
# fetches the lock's crates for the release's target (`cargo fetch --locked`), the one step that
# uses the network, and a no-op when they are all here. With CARGO_FLAGS=--offline the fetch only
# checks that they are.
#
# Needs no NeMo prefix and no Homebrew: nothing is built but the generator (ink-ffi without its
# engine features). A crate without its licence text, or an override no crate needs, fails with
# the crate named and writes nothing (core/crates/ink-ffi/notices/overrides.txt says what to do).
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
fail() { echo "rust-notices: $*" >&2; exit 1; }

mode=()
case "${1:-}" in
  "") ;;
  --check) mode=(--check) ;;
  *) fail "usage: rust-notices.sh [--check]" ;;
esac
[ $# -le 1 ] || fail "usage: rust-notices.sh [--check]"

cd "$root/core"
# shellcheck disable=SC2086 # CARGO_FLAGS is split on purpose
cargo fetch --locked --target aarch64-apple-darwin ${CARGO_FLAGS:-}
# shellcheck disable=SC2086
cargo run --locked --quiet -p ink-ffi --bin ink-notices ${CARGO_FLAGS:-} -- "${mode[@]+"${mode[@]}"}"
