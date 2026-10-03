#!/usr/bin/env bash
# windows/scripts/release-version.sh: which tags and dry-run versions a Windows release accepts,
# what it prints, and the notices and the core's version it waits for. Runs anywhere bash does (the Mac's assert.sh):
#
#   bash windows/scripts/tests/test-release-version.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/../../../mac/scripts/tests/assert.sh"
script="$here/../release-version.sh"

work="$(mktemp -d "${TMPDIR:-/tmp}/ink-win-release-version.XXXXXX")"
trap 'rm -rf "$work"' EXIT
printf 'foo 1.0.0 MIT.txt verified=2026-10-04 why\n' >"$work/done.txt"
printf 'winfoo verified=no the LICENSE upstream\n' >"$work/open.txt"
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
run "... and its installer" 0 "setup=Inkwell_1.2.3_x64-setup.exe" tag v1.2.3
run "... and its update package" 0 "package=InkwellApp-1.2.3-full.nupkg" tag v1.2.3
run "... and its checksums" 0 "sums=Inkwell_1.2.3_windows-sha256.txt" tag v1.2.3
core_at 1.0.0
run "a dry run's version" 0 "version=1.0.0" dry-run 1.0.0

for bad in v0.2.11 v2.0.0 v1.2 v1.2.3.4 v1.2.3-rc.1 v1.02.3 1.2.3 "v1.2.3 " "v1.2.3;echo" ""; do
  run "tag '$bad' is refused" 1 "a Windows release tag is v1.X.Y" tag "$bad"
done
for bad in 1.2 v1.2.3 1.2.3-beta 01.2.3 "1.2.3\$(id)" ""; do
  run "dry-run version '$bad' is refused" 1 "a dry run's version is X.Y.Z" dry-run "$bad"
done
run "an unknown kind" 1 "unknown kind" nightly 1.2.3

# A tag waits for every notice to be compared with its upstream file; a dry run reports.
INK_NOTICES_FILES="$work/done.txt:$work/open.txt"
run "a tag while a Windows notice is unchecked" 1 "every licence notice compared with its upstream file" tag v1.0.0
run "... names it" 1 "winfoo" tag v1.0.0
run "a dry run while a notice is unchecked" 0 "version=1.0.0" dry-run 1.0.0
out="$(/bin/bash "$script" dry-run 1.0.0 2>/dev/null)"
assert_absent "the dry run's report stays off stdout (the workflow's outputs)" "$out" "winfoo"

# A release tag waits for the core to say the same version (About shows it); a dry run reports.
INK_NOTICES_FILES="$work/done.txt"
core_at 1.0.0
run "a tag the core's version differs from" 1 "core/Cargo.toml says 1.0.0, the release is 1.0.1" tag v1.0.1
run "... says why it stopped" 1 "a release tag needs the core at the release's version first" tag v1.0.1
run "a tag the core's version matches" 0 "version=1.0.0" tag v1.0.0
run "a dry run the core's version differs from" 0 "version=1.0.1" dry-run 1.0.1
run "... says so" 0 "core/Cargo.toml says 1.0.0, the release is 1.0.1" dry-run 1.0.1
out="$(/bin/bash "$script" dry-run 1.0.1 2>/dev/null)"
assert_absent "the dry run's core report stays off stdout (the workflow's outputs)" "$out" "core/Cargo.toml"

# Without the override, the Windows-only list is among those read: the total counts its lines,
# whether they are verified yet or not (a tag needs them verified, so no line may be assumed open).
unset INK_NOTICES_FILES
entries() { cat "$@" | grep -cvE '^(#|$)' || true; }
windows_list="$here/../../Inkwell.Core/Screens/About/composed-notices.txt"
windows_entries="$(entries "$windows_list")"
[ "$windows_entries" -gt 0 ] && pass "the Windows-only list has notices" || flunk "the Windows-only list has no notices"
all="$(entries "$here/../../../core/crates/ink-ffi/notices/overrides.txt" "$here/../../../core/crates/ink-ffi/notices/velopack/overrides.txt" "$here/../../../mac/composed-notices.txt" "$windows_list")"
out="$(/bin/bash "$script" dry-run 1.0.0 2>&1)"
assert_contains "the real lists include the Windows-only notices" "$out" " $all notice(s)"

finish
