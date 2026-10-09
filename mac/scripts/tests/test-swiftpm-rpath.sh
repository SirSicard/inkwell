#!/usr/bin/env bash
# Real Mach-O fixture: remove the verified SwiftPM path, retain unknown absolute paths so the
# bundle checker still refuses them. No fixture is executed or signed.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/assert.sh"
. "$here/../lib/swiftpm-rpath.sh"
. "$here/../lib/bundle-check.sh"
work="$(mktemp -d "${TMPDIR:-/tmp}/ink-swiftpm-rpath-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT
mac_dir="$work/mac"
bin_dir="$mac_dir/.build/out/Products/Debug"
mkdir -p "$bin_dir/PackageFrameworks" "$work/fixture.app/Contents/MacOS"
printf 'int main(void) { return 0; }\n' >"$work/main.c"
executable="$work/fixture.app/Contents/MacOS/App"
xcrun clang -mmacosx-version-min=26.0 -Wl,-headerpad_max_install_names \
  -Wl,-rpath,"$bin_dir/PackageFrameworks" -Wl,-rpath,@executable_path/../Frameworks \
  -o "$executable" "$work/main.c"
status=0
check_bundle_linkage "$work/fixture.app" 26.0 >"$work/check" 2>&1 || status=$?
assert_status "the original SwiftPM absolute rpath is refused" 1 "$status"
remove_swiftpm_framework_rpath "$executable" "$mac_dir" "$bin_dir"
paths="$(otool -l "$executable")"
assert_absent "the exact build-local path is removed" "$paths" "$bin_dir/PackageFrameworks"
assert_contains "the bundled framework search path stays" "$paths" '@executable_path/../Frameworks'
status=0
check_bundle_linkage "$work/fixture.app" 26.0 >"$work/check" 2>&1 || status=$?
assert_status "the repaired bundle passes the unchanged linkage check" 0 "$status"
remove_swiftpm_framework_rpath "$executable" "$mac_dir" "$bin_dir"
pass "repeated removal is harmless"
install_name_tool -add_rpath /opt/homebrew/lib "$executable"
remove_swiftpm_framework_rpath "$executable" "$mac_dir" "$bin_dir"
status=0
check_bundle_linkage "$work/fixture.app" 26.0 >"$work/check" 2>&1 || status=$?
assert_status "unknown absolute paths still fail linkage" 1 "$status"
paths="$(otool -l "$executable")"
assert_contains "unknown absolute paths are preserved for refusal" "$paths" /opt/homebrew/lib
mkdir -p "$work/outside/PackageFrameworks"
ln -s "$work/outside" "$mac_dir/.build/escaped"
status=0
remove_swiftpm_framework_rpath "$executable" "$mac_dir" "$mac_dir/.build/escaped" >"$work/check" 2>&1 || status=$?
assert_status "a build path escaping through a symlink is refused" 1 "$status"
finish
