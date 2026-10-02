#!/usr/bin/env bash
# release-version.sh: which tags and dry-run versions a Mac release accepts, and what it prints.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/assert.sh"
script="$here/../release-version.sh"

work="$(mktemp -d "${TMPDIR:-/tmp}/ink-release-version.XXXXXX")"
trap 'rm -rf "$work"' EXIT
# The notices a release waits for (notices-verified.sh): all compared with upstream here, so the
# version rules are tested on their own; the gate has its own cases below.
printf 'foo 1.0.0 MIT.txt verified=2026-10-04 why\n' >"$work/done.txt"
printf 'foo 1.0.0 MIT.txt verified=no why\n' >"$work/open.txt"
export INK_NOTICES_FILES="$work/done.txt"
# The core's version (core-version.sh): the release's own here, so the version rules are tested on
# their own; the gate has its own cases below.
core_at() { printf '[workspace.package]\nversion = "%s"\n' "$1" >"$work/Cargo.toml"; }
export INK_CORE_MANIFEST="$work/Cargo.toml"

# run <label> <expected status> <expected text> <arguments...>
run() {
  local label=$1 want=$2 text=$3 out status=0
  shift 3
  out="$(/bin/bash "$script" "$@" 2>&1)" || status=$?
  assert_status "$label" "$want" "$status"
  assert_contains "$label: says" "$out" "$text"
}

core_at 1.2.3
run "a v1 tag" 0 "version=1.2.3" tag v1.2.3
run "... and its dmg" 0 "dmg=Inkwell_1.2.3_aarch64.dmg" tag v1.2.3
run "... and its build manifest" 0 "manifest=Inkwell_1.2.3_build-manifest.txt" tag v1.2.3
core_at 1.0.0
run "v1.0.0" 0 "version=1.0.0" tag v1.0.0
run "a dry run's version" 0 "version=0.0.1" dry-run 0.0.1

for bad in v0.2.11 v2.0.0 v1.2 v1.2.3.4 v1.2.3-rc.1 v1.02.3 1.2.3 "v1.2.3 " "v1.2.3;echo" ""; do
  run "tag '$bad' is refused" 1 "a Mac release tag is v1.X.Y" tag "$bad"
done
for bad in 1.2 v1.2.3 1.2.3-beta 01.2.3 "1.2.3\$(id)" ""; do
  run "dry-run version '$bad' is refused" 1 "a dry run's version is X.Y.Z" dry-run "$bad"
done
run "an unknown kind" 1 "unknown kind" nightly 1.2.3

# A release tag waits for every notice to be compared with its upstream file; a dry run reports.
INK_NOTICES_FILES="$work/open.txt"
run "a tag while a notice is unchecked" 1 "every licence notice compared with its upstream file" tag v1.0.0
run "... names it" 1 "foo 1.0.0" tag v1.0.0
run "a dry run while a notice is unchecked" 0 "version=1.0.0" dry-run 1.0.0
run "... says so" 0 "not yet compared with its upstream file" dry-run 1.0.0
out="$(/bin/bash "$script" dry-run 1.0.0 2>/dev/null)"
assert_absent "the dry run's report stays off stdout (the workflow's outputs)" "$out" "foo 1.0.0"
INK_NOTICES_FILES="$work/done.txt"

# A release tag waits for the core to say the same version (About shows it); a dry run reports.
core_at 1.0.0
run "a tag the core's version differs from" 1 "core/Cargo.toml says 1.0.0, the release is 1.0.1" tag v1.0.1
run "a dry run the core's version differs from" 0 "version=1.0.1" dry-run 1.0.1
run "... says so" 0 "core/Cargo.toml says 1.0.0, the release is 1.0.1" dry-run 1.0.1
out="$(/bin/bash "$script" dry-run 1.0.1 2>/dev/null)"
assert_absent "the dry run's report stays off stdout (the workflow's outputs)" "$out" "core/Cargo.toml"

finish
