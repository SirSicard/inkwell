#!/usr/bin/env bash
# Whether the core says the version a release builds: core/Cargo.toml's [workspace.package]
# version, which every crate takes and the core reports in core.ready (About's "core X.Y.Z", on the
# Mac and on Windows). The tag gives the app its version; nothing gives the core its, so a release
# tag waits until the two agree. release-version.sh runs this for a tag, and with --warn for a dry
# run; the Windows release's build job runs it the same way.
#
#   mac/scripts/core-version.sh X.Y.Z          fails unless the core's version is X.Y.Z
#   mac/scripts/core-version.sh --warn X.Y.Z   says so and passes (the dry run)
#
# Either way it fails when the manifest has no workspace version. Everything it prints goes to
# stderr: release-version.sh's stdout is the workflow's outputs. INK_CORE_MANIFEST replaces
# core/Cargo.toml, for the tests.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
fail() { echo "core-version: $*" >&2; exit 1; }
usage="usage: core-version.sh [--warn] X.Y.Z"

warn=0
if [ "${1:-}" = --warn ]; then
  warn=1
  shift
fi
[ $# -eq 1 ] && [ -n "$1" ] || fail "$usage"
release="$1"

manifest="${INK_CORE_MANIFEST:-$root/core/Cargo.toml}"
[ -r "$manifest" ] || fail "cannot read $manifest"
# The `version = "..."` line of the [workspace.package] table, and of no other: a dependency's
# table carries versions too.
core="$(awk '
  /^\[/ { in_package = ($0 == "[workspace.package]"); next }
  in_package && /^version[ \t]*=/ {
    sub(/^version[ \t]*=[ \t]*"/, ""); sub(/".*$/, ""); print; exit
  }
' "$manifest")"
[ -n "$core" ] || fail "$manifest has no [workspace.package] version"

if [ "$core" = "$release" ]; then
  echo "core-version: core $core, as the release" >&2
  exit 0
fi
message="core/Cargo.toml says $core, the release is $release: set [workspace.package] version to $release, then update core/Cargo.lock (cargo update --workspace) and the Rust notices (mac/scripts/rust-notices.sh; ink-notices --windows), in one pull request (docs/RELEASING.md)"
if [ "$warn" -eq 1 ]; then
  echo "core-version: $message. A release tag would stop here." >&2
  exit 0
fi
fail "$message"
