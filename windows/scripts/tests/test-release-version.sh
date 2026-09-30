#!/usr/bin/env bash
# windows/scripts/release-version.sh: which tags and dry-run versions a Windows release accepts,
# what it prints, and the notices it waits for. Runs anywhere bash does (the Mac's assert.sh):
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

# run <label> <expected status> <expected text> <arguments...>
run() {
  local label=$1 want=$2 text=$3 out status=0
  shift 3
  out="$(/bin/bash "$script" "$@" 2>&1)" || status=$?
  assert_status "$label" "$want" "$status"
  assert_contains "$label: says" "$out" "$text"
}

run "a v1 tag" 0 "version=1.2.3" tag v1.2.3 x64
run "... and its x64 installer" 0 "setup=Inkwell_1.2.3_x64-setup.exe" tag v1.2.3 x64
run "... and its x64 update package" 0 "package=InkwellApp-1.2.3-full.nupkg" tag v1.2.3 x64
run "... on x64's channel" 0 "feed=releases.win.json" tag v1.2.3 x64
run "... and its x64 checksums" 0 "sums=Inkwell_1.2.3_windows-x64-sha256.txt" tag v1.2.3 x64
run "... and its ARM64 installer" 0 "setup=Inkwell_1.2.3_arm64-setup.exe" tag v1.2.3 arm64
run "... and its ARM64 update package" 0 "package=InkwellApp-1.2.3-win-arm64-full.nupkg" tag v1.2.3 arm64
run "... on ARM64's own channel" 0 "feed=releases.win-arm64.json" tag v1.2.3 arm64
run "... and its ARM64 checksums" 0 "sums=Inkwell_1.2.3_windows-arm64-sha256.txt" tag v1.2.3 arm64
run "a dry run's version" 0 "version=1.0.0" dry-run 1.0.0 arm64

# The two architectures' files share a release: no name may be the same.
x64="$(/bin/bash "$script" tag v1.2.3 x64 2>/dev/null | grep -v '^version=' | cut -d= -f2)"
arm64="$(/bin/bash "$script" tag v1.2.3 arm64 2>/dev/null | grep -v '^version=' | cut -d= -f2)"
[ "$(printf '%s\n%s\n' "$x64" "$arm64" | sort | uniq -d)" = "" ] && [ "$(wc -l <<<"$x64")" -eq 4 ] \
  && pass "x64's and ARM64's files are named apart" || flunk "a file name is shared: $x64 / $arm64"

for bad in v0.2.11 v2.0.0 v1.2 v1.2.3.4 v1.2.3-rc.1 v1.02.3 1.2.3 "v1.2.3 " "v1.2.3;echo" ""; do
  run "tag '$bad' is refused" 1 "a Windows release tag is v1.X.Y" tag "$bad" x64
done
for bad in 1.2 v1.2.3 1.2.3-beta 01.2.3 "1.2.3\$(id)" ""; do
  run "dry-run version '$bad' is refused" 1 "a dry run's version is X.Y.Z" dry-run "$bad" x64
done
for bad in x86 ARM64 amd64 aarch64 "" "x64;echo"; do
  run "architecture '$bad' is refused" 1 "the architecture is x64 or arm64" tag v1.2.3 "$bad"
done
run "no architecture" 1 "usage" tag v1.2.3
run "an unknown kind" 1 "unknown kind" nightly 1.2.3 x64

# A tag waits for every notice to be compared with its upstream file; a dry run reports.
INK_NOTICES_FILES="$work/done.txt:$work/open.txt"
run "a tag while a Windows notice is unchecked" 1 "every licence notice compared with its upstream file" tag v1.0.0 x64
run "... names it" 1 "winfoo" tag v1.0.0 x64
run "... on ARM64 too" 1 "winfoo" tag v1.0.0 arm64
run "a dry run while a notice is unchecked" 0 "version=1.0.0" dry-run 1.0.0 arm64
out="$(/bin/bash "$script" dry-run 1.0.0 arm64 2>/dev/null)"
assert_absent "the dry run's report stays off stdout (the workflow's outputs)" "$out" "winfoo"

# Without the override, the Windows-only list is among those read: the total counts its lines,
# whether they are verified yet or not (a tag needs them verified, so no line may be assumed open).
unset INK_NOTICES_FILES
entries() { cat "$@" | grep -cvE '^(#|$)' || true; }
windows_list="$here/../../Inkwell.Core/Screens/About/composed-notices.txt"
windows_entries="$(entries "$windows_list")"
[ "$windows_entries" -gt 0 ] && pass "the Windows-only list has notices" || flunk "the Windows-only list has no notices"
all="$(entries "$here/../../../core/crates/ink-ffi/notices/overrides.txt" "$here/../../../mac/composed-notices.txt" "$windows_list")"
out="$(/bin/bash "$script" dry-run 1.0.0 x64 2>&1)"
assert_contains "the real lists include the Windows-only notices" "$out" " $all notice(s)"

finish
