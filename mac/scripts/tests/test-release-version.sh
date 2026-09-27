#!/usr/bin/env bash
# release-version.sh: which tags and dry-run versions a Mac release accepts, and what it prints.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/assert.sh"
script="$here/../release-version.sh"

# run <label> <expected status> <expected text> <arguments...>
run() {
  local label=$1 want=$2 text=$3 out status=0
  shift 3
  out="$(/bin/bash "$script" "$@" 2>&1)" || status=$?
  assert_status "$label" "$want" "$status"
  assert_contains "$label: says" "$out" "$text"
}

run "a v1 tag" 0 "version=1.2.3" tag v1.2.3
run "... and its dmg" 0 "dmg=Inkwell_1.2.3_aarch64.dmg" tag v1.2.3
run "... and its build manifest" 0 "manifest=Inkwell_1.2.3_build-manifest.txt" tag v1.2.3
run "v1.0.0" 0 "version=1.0.0" tag v1.0.0
run "a dry run's version" 0 "version=0.0.1" dry-run 0.0.1

for bad in v0.2.11 v2.0.0 v1.2 v1.2.3.4 v1.2.3-rc.1 v1.02.3 1.2.3 "v1.2.3 " "v1.2.3;echo" ""; do
  run "tag '$bad' is refused" 1 "a Mac release tag is v1.X.Y" tag "$bad"
done
for bad in 1.2 v1.2.3 1.2.3-beta 01.2.3 "1.2.3\$(id)" ""; do
  run "dry-run version '$bad' is refused" 1 "a dry run's version is X.Y.Z" dry-run "$bad"
done
run "an unknown kind" 1 "unknown kind" nightly 1.2.3

finish
