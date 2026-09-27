#!/usr/bin/env bash
# build-mac.sh checks the app's signed entitlements against an allow-list: exactly the keys of
# mac/Inkwell.entitlements with their values, plus disable-library-validation for an ad-hoc build
# only. A missing key is a feature the hardened runtime denies silently; an extra one is a
# capability nobody reviewed.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/assert.sh"
. "$here/../lib/entitlements-check.sh"

dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT

# plist <file> <key=value>...: an entitlements plist (values true or false, or string:<text> for a
# string).
plist() {
  local file="$1" pair value
  shift
  {
    echo '<?xml version="1.0" encoding="UTF-8"?>'
    echo '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">'
    echo '<plist version="1.0"><dict>'
    for pair in "$@"; do
      value="${pair#*=}"
      case "$value" in
        string:*) echo "  <key>${pair%%=*}</key><string>${value#string:}</string>" ;;
        *) echo "  <key>${pair%%=*}</key><$value/>" ;;
      esac
    done
    echo '</dict></plist>'
  } >"$file"
}

mic=com.apple.security.device.audio-input
cal=com.apple.security.personal-information.calendars
dlv=com.apple.security.cs.disable-library-validation
plist "$dir/expected.plist" "$mic=true" "$cal=true"

# run <label> <adhoc> <expected status> [signed key=value...]: checks a signed set.
run() {
  local label="$1" adhoc="$2" want="$3"
  shift 3
  plist "$dir/signed.plist" "$@"
  set +e
  out="$(check_entitlements "$dir/signed.plist" "$dir/expected.plist" "$adhoc" 2>&1)"
  status=$?
  set -e
  assert_status "$label" "$want" "$status"
}

run "exactly the expected set (Developer ID)" 0 0 "$mic=true" "$cal=true"
run "the expected set plus library validation off (ad-hoc)" 1 0 "$mic=true" "$cal=true" "$dlv=true"

run "the calendar entitlement missing" 0 1 "$mic=true"
assert_contains "names the missing key" "$out" "$cal is missing"
run "the microphone entitlement missing" 1 1 "$cal=true" "$dlv=true"
assert_contains "names the missing key (ad-hoc)" "$out" "$mic is missing"

run "a key nobody asked for" 0 1 "$mic=true" "$cal=true" "com.apple.security.device.camera=true"
assert_contains "names the extra key" "$out" "com.apple.security.device.camera was not asked for"

run "library validation off in a Developer ID build" 0 1 "$mic=true" "$cal=true" "$dlv=true"
assert_contains "names library validation" "$out" "$dlv was not asked for"
run "an ad-hoc build without library validation off" 1 1 "$mic=true" "$cal=true"
assert_contains "names it missing" "$out" "$dlv is missing"

# PlistBuddy prints <true/> and <string>true</string> alike; the hardened runtime honours only the
# boolean, so a string would pass by its text and grant nothing.
run "a true written as a string" 0 1 "$mic=true" "$cal=string:true"
assert_contains "names the wrong type" "$out" "$cal is string true, expected true"
run "library validation off written as a string (ad-hoc)" 1 1 "$mic=true" "$cal=true" "$dlv=string:true"
assert_contains "names its wrong type" "$out" "$dlv is string true, expected true"

run "an expected key signed false" 0 1 "$mic=true" "$cal=false"
assert_contains "names the wrong value" "$out" "$cal is false, expected true"

set +e
out="$(check_entitlements "$dir/none.plist" "$dir/expected.plist" 0 2>&1)"
status=$?
set -e
assert_status "unreadable signed entitlements" 1 "$status"

# The real file: the keys build-mac.sh signs into every build.
keys="$(entitlement_keys "$here/../../Inkwell.entitlements")"
assert_contains "Inkwell.entitlements: audio-input" "$keys" "$mic"
assert_contains "Inkwell.entitlements: calendars" "$keys" "$cal"
assert_absent "Inkwell.entitlements: library validation stays on" "$keys" "$dlv"

finish
