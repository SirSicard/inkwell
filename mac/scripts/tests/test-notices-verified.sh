#!/usr/bin/env bash
# notices-verified.sh: a release tag waits until every notice written without its upstream licence
# file has been compared with it (a date in its verified= marker); the dry run only reports.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/assert.sh"
script="$here/../notices-verified.sh"
work="$(mktemp -d "${TMPDIR:-/tmp}/ink-notices-verified.XXXXXX")"
trap 'rm -rf "$work"' EXIT

# run <label> <expected status> <expected text> <files, colon-separated> <arguments...>
run() {
  local label=$1 want=$2 text=$3 files=$4 out status=0
  shift 4
  out="$(INK_NOTICES_FILES="$files" /bin/bash "$script" "$@" 2>&1)" || status=$?
  assert_status "$label" "$want" "$status"
  assert_contains "$label: says" "$out" "$text"
}

cat >"$work/done-overrides.txt" <<'T'
# a comment
foo 1.0.0 MIT.txt verified=2026-10-04 the MIT licence's text

bar 2.0.0 Apache-2.0.txt verified=2026-10-05 the Apache licence
T
cat >"$work/done-composed.txt" <<'T'
protobuf-lite verified=2026-10-04 SentencePiece's copy
T
cat >"$work/open-overrides.txt" <<'T'
foo 1.0.0 MIT.txt verified=2026-10-04 the MIT licence's text
bar 2.0.0 Apache-2.0.txt verified=no the Apache licence
T
cat >"$work/open-composed.txt" <<'T'
darts-clone verified=no SentencePiece's copy
T
printf 'foo 1.0.0 MIT.txt verified=soon why\n' >"$work/bad-marker.txt"
printf 'foo 1.0.0 MIT.txt why\n' >"$work/no-marker.txt"

done="$work/done-overrides.txt:$work/done-composed.txt"
open="$work/open-overrides.txt:$work/open-composed.txt"
run "all compared with upstream" 0 "3 notice(s), each compared" "$done"
run "one not compared" 1 "bar 2.0.0" "$open"
run "... names each" 1 "darts-clone" "$open"
run "... says what to do" 1 "RELEASING.md" "$open"
run "the dry run only reports" 0 "not yet compared with its upstream file" "$open" --warn
run "... naming them" 0 "bar 2.0.0" "$open" --warn
run "a marker that is not a date" 1 "verified=soon" "$work/bad-marker.txt" --warn
run "a line without a marker" 1 "no verified= marker" "$work/no-marker.txt" --warn
run "a missing file" 1 "cannot read" "$work/absent.txt" --warn
run "an unknown argument" 1 "usage" "$done" --tag
# The repository's own lists parse (whatever they say about upstream).
out="$(/bin/bash "$script" --warn 2>&1)" && status=0 || status=$?
assert_status "the repository's lists are well formed" 0 "$status"
assert_contains "... and cover the overrides" "$out" "notice(s)"

finish
