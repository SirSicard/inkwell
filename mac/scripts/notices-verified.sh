#!/usr/bin/env bash
# Whether every licence notice written without its upstream file on hand has since been compared
# with it: the Rust crates' supplied texts (core/crates/ink-ffi/notices/overrides.txt) and the
# notices Notices.swift composes (mac/composed-notices.txt). Each line of both carries
# `verified=no` or `verified=YYYY-MM-DD`. A release tag waits for a date on every line:
# release-version.sh runs this for a tag, and with --warn for a dry run.
#
#   mac/scripts/notices-verified.sh          fails while any line reads verified=no
#   mac/scripts/notices-verified.sh --warn   lists them and passes (the dry run)
#
# Either way it fails on a line without a marker or with one that is neither `no` nor a date.
# Everything it prints goes to stderr: release-version.sh's stdout is the workflow's outputs.
# INK_NOTICES_FILES (colon-separated) replaces the two lists, for the tests.
set -euo pipefail
# The lines are split into words below: no word may expand as a file pattern.
set -f

root="$(cd "$(dirname "$0")/../.." && pwd)"
fail() { echo "notices-verified: $*" >&2; exit 1; }

warn=0
case "${1:-}" in
  "") ;;
  --warn) warn=1 ;;
  *) fail "usage: notices-verified.sh [--warn]" ;;
esac
[ $# -le 1 ] || fail "usage: notices-verified.sh [--warn]"

files="${INK_NOTICES_FILES:-$root/core/crates/ink-ffi/notices/overrides.txt:$root/mac/composed-notices.txt}"
IFS=: read -r -a lists <<<"$files"

total=0
open=()
for list in "${lists[@]}"; do
  [ -r "$list" ] || fail "cannot read $list"
  content="$(cat "$list")"
  n=0
  while IFS= read -r line; do
    n=$((n + 1))
    case "$line" in "" | "#"*) continue ;; esac
    marker=""
    entry=""
    for word in $line; do
      case "$word" in
        verified=*) marker="${word#verified=}"; break ;;
        *) entry="${entry:+$entry }$word" ;;
      esac
    done
    [ -n "$marker" ] || [[ "$line" == *"verified="* ]] || fail "$(basename "$list") line $n ($entry) has no verified= marker"
    total=$((total + 1))
    if [ "$marker" = no ]; then
      open+=("$(basename "$list"): $entry")
    elif ! [[ "$marker" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]]; then
      fail "$(basename "$list") line $n ($entry): verified=$marker is neither no nor a date (YYYY-MM-DD)"
    fi
  done <<<"$content"
done

if [ ${#open[@]} -eq 0 ]; then
  echo "notices-verified: $total notice(s), each compared with its upstream licence file" >&2
  exit 0
fi
{
  echo "notices-verified: ${#open[@]} of $total notice(s) not yet compared with its upstream file:"
  printf '  %s\n' "${open[@]}"
  echo "Compare each, replace a text that differs, and set its date (docs/RELEASING.md, \"Release day\")."
} >&2
[ "$warn" = 1 ] || exit 1
