#!/usr/bin/env bash
# core-version.sh: a release tag waits until the core says the version it is built as
# (core/Cargo.toml's workspace version, which About shows as "core X.Y.Z"); a dry run only reports.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/assert.sh"
script="$here/../core-version.sh"
work="$(mktemp -d "${TMPDIR:-/tmp}/ink-core-version.XXXXXX")"
trap 'rm -rf "$work"' EXIT

# run <label> <expected status> <expected text> <manifest> <arguments...>
run() {
  local label=$1 want=$2 text=$3 manifest=$4 out status=0
  shift 4
  out="$(INK_CORE_MANIFEST="$manifest" /bin/bash "$script" "$@" 2>&1)" || status=$?
  assert_status "$label" "$want" "$status"
  assert_contains "$label: says" "$out" "$text"
}

# The workspace's own layout: a crate's dependency carries a version too, and must not be read.
cat >"$work/core.toml" <<'T'
[workspace]
resolver = "3"

[workspace.package]
version = "1.2.3"
edition = "2024"

[workspace.dependencies]
ink-core = { path = "crates/ink-core" }
rubato = { version = "0.16.2" }
T
cat >"$work/no-version.toml" <<'T'
[workspace.package]
edition = "2024"

[workspace.dependencies]
version = "9.9.9"
T

run "the core at the release's version" 0 "core 1.2.3" "$work/core.toml" 1.2.3
run "the core at another version" 1 "core/Cargo.toml says 1.2.3, the release is 1.2.4" "$work/core.toml" 1.2.4
run "... says how to fix it" 1 "Cargo.lock" "$work/core.toml" 1.2.4
run "a dry run at another version goes on" 0 "core/Cargo.toml says 1.2.3, the release is 1.0.0" \
  "$work/core.toml" --warn 1.0.0
out="$(INK_CORE_MANIFEST="$work/core.toml" /bin/bash "$script" --warn 1.0.0 2>/dev/null)"
assert_absent "the report stays off stdout (release-version.sh's stdout is the workflow's outputs)" "$out" "1.2.3"
run "no workspace version" 1 "no [workspace.package] version" "$work/no-version.toml" 1.2.3
run "... not even for a dry run" 1 "no [workspace.package] version" "$work/no-version.toml" --warn 1.2.3
run "an unreadable manifest" 1 "cannot read" "$work/missing.toml" 1.2.3
run "no version given" 1 "usage" "$work/core.toml"

# The real manifest has a version of the 1.x line: About must never read "core 0.0.0" again.
out="$(/bin/bash "$script" 0.0.0 2>&1 || true)"
assert_absent "the committed core is not 0.0.0" "$out" "core 0.0.0"

finish
