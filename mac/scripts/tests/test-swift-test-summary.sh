#!/usr/bin/env bash
# swift-test-summary.sh adds up the XCTest bundles: `swift test` runs each bundle on its own, and
# each prints its own "Executed N tests" twice (the bundle, then "All tests").
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/assert.sh"
summary="$here/../swift-test-summary.sh"
work="$(mktemp -d "${TMPDIR:-/tmp}/ink-summary-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT

# The shape `swift test` prints on macOS (two bundles, trimmed).
cat >"$work/pass.log" <<'LOG'
Test Suite 'All tests' started at 2026-09-27 09:41:17.551.
Test Suite 'InkwellTests.xctest' started at 2026-09-27 09:41:17.555.
Test Suite 'DataLocationTests' started at 2026-09-27 09:41:17.555.
	 Executed 3 tests, with 0 failures (0 unexpected) in 0.001 (0.001) seconds
Test Suite 'InkwellTests.xctest' passed at 2026-09-27 09:41:17.600.
	 Executed 11 tests, with 0 failures (0 unexpected) in 0.044 (0.045) seconds
Test Suite 'All tests' passed at 2026-09-27 09:41:17.600.
	 Executed 11 tests, with 0 failures (0 unexpected) in 0.044 (0.049) seconds
Test Suite 'All tests' started at 2026-09-27 09:41:17.730.
Test Suite 'InkBridgeTests.xctest' started at 2026-09-27 09:41:17.735.
Test Suite 'SmokeTests' passed at 2026-09-27 09:41:18.496.
	 Executed 1 test, with 0 failures (0 unexpected) in 0.173 (0.173) seconds
Test Suite 'InkBridgeTests.xctest' passed at 2026-09-27 09:41:18.496.
	 Executed 24 tests, with 0 failures (0 unexpected) in 0.760 (0.761) seconds
Test Suite 'All tests' passed at 2026-09-27 09:41:18.496.
	 Executed 24 tests, with 0 failures (0 unexpected) in 0.760 (0.766) seconds
✔ Test run with 0 tests in 0 suites passed after 0.001 seconds.
LOG
status=0
out="$(/bin/bash "$summary" "$work/pass.log")" || status=$?
assert_status "two bundles" 0 "$status"
assert_contains "the total is the sum, not the last bundle's" "$out" "35 tests in 2 bundles"
assert_contains "each bundle is named" "$out" "InkwellTests 11, InkBridgeTests 24"

sed -e "s/'InkBridgeTests.xctest' passed/'InkBridgeTests.xctest' failed/" \
  -e 's/Executed 24 tests, with 0 failures (0 unexpected)/Executed 24 tests, with 2 failures (0 unexpected)/' \
  "$work/pass.log" >"$work/fail.log"
status=0
out="$(/bin/bash "$summary" "$work/fail.log")" || status=$?
assert_status "a failing bundle fails the summary" 1 "$status"
assert_contains "failures are counted" "$out" "2 failures"

# A bundle with skipped tests says so in its count line ("with 3 tests skipped and 0 failures").
sed -e 's/Executed 24 tests, with 0 failures (0 unexpected)/Executed 24 tests, with 3 tests skipped and 0 failures (0 unexpected)/' \
  "$work/pass.log" >"$work/skipped.log"
status=0
out="$(/bin/bash "$summary" "$work/skipped.log")" || status=$?
assert_status "skipped tests are not failures" 0 "$status"
assert_contains "a bundle with skipped tests is still counted" "$out" "35 tests in 2 bundles"
assert_contains "and named with its own count" "$out" "InkwellTests 11, InkBridgeTests 24"
assert_contains "the skipped are reported" "$out" "0 failures, 3 skipped"
sed -e 's/Executed 24 tests, with 0 failures (0 unexpected)/Executed 24 tests, with 1 test skipped and 2 failures (0 unexpected)/' \
  "$work/pass.log" >"$work/skipfail.log"
status=0
out="$(/bin/bash "$summary" "$work/skipfail.log")" || status=$?
assert_status "a failure beside a skip fails" 1 "$status"
assert_contains "failures are read after the skips" "$out" "2 failures, 1 skipped"

printf 'error: build failed\n' >"$work/none.log"
status=0
out="$(/bin/bash "$summary" "$work/none.log")" || status=$?
assert_status "no bundle ran" 1 "$status"
assert_contains "no bundle: says so" "$out" "no XCTest bundle"

finish
