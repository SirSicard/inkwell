#!/usr/bin/env bash
# Adds up a `swift test` log's XCTest bundles. SwiftPM runs each bundle in a process of its own,
# and each prints its own "Executed N tests" (for the bundle, then again for "All tests"), so the
# last count in the log is one bundle's, not the run's.
#
#   mac/scripts/swift-test-summary.sh <swift test log>
#
# Prints "Swift tests: N tests in K bundles (A n, B m), F failures". Exits 1 on any failure, or
# when the log holds no finished bundle (a build error, or a format this script no longer reads).
set -euo pipefail

log="${1:?usage: $0 <swift test log>}"
awk '
  # A bundle has finished; its count is on the next line.
  /^Test Suite .*\.xctest. (passed|failed) at / {
    name = $0
    sub(/^Test Suite ./, "", name)
    sub(/\.xctest.*$/, "", name)
    pending = name
    next
  }
  pending != "" && /Executed [0-9]+ tests?, with [0-9]+ failures?/ {
    match($0, /Executed [0-9]+/); tests = substr($0, RSTART + 9, RLENGTH - 9) + 0
    match($0, /with [0-9]+/); fails = substr($0, RSTART + 5, RLENGTH - 5) + 0
    total += tests; failures += fails; bundles++
    list = list (list == "" ? "" : ", ") pending " " tests
    pending = ""
  }
  END {
    if (bundles == 0) { print "Swift tests: no XCTest bundle finished in the log"; exit 1 }
    printf "Swift tests: %d tests in %d bundles (%s), %d failures\n", total, bundles, list, failures
    exit failures > 0 ? 1 : 0
  }
' "$log"
