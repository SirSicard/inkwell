#!/usr/bin/env bash
# The release scripts refuse to produce something unsigned, unnotarisable or unchecked, before
# they build, sign or upload anything. (What they produce is checked for real by the release
# workflow and, short of notarisation, by hand: docs/RELEASING.md.) Offline, no identity needed.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/assert.sh"
scripts="$here/.."

work="$(mktemp -d "${TMPDIR:-/tmp}/ink-refusal-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/Inkwell.app/Contents"
touch "$work/some.zip" "$work/Inkwell.dmg"

# run <label> <expected text> <environment and command...>: must fail, saying why.
run() {
  local label=$1 text=$2 out status=0
  shift 2
  out="$(env -u INK_SIGN_IDENTITY -u NOTARY_PROFILE -u SPARKLE_BIN "$@" 2>&1 </dev/null)" || status=$?
  if [ "$status" = 0 ]; then flunk "$label: it succeeded"; else pass "$label"; fi
  assert_contains "$label: says why" "$out" "$text"
}

run "a timestamped build without an identity" "--timestamp needs INK_SIGN_IDENTITY" \
  /bin/bash "$scripts/build-mac.sh" --skip-core --timestamp
run "a dmg without an identity" "the dmg would go out unsigned" \
  /bin/bash "$scripts/package-dmg.sh" "$work/Inkwell.app" "$work/out.dmg"
run "a dmg named otherwise" "the output must end in .dmg" \
  INK_SIGN_IDENTITY=x /bin/bash "$scripts/package-dmg.sh" "$work/Inkwell.app" "$work/out.img"
run "notarising without credentials" "NOTARY_PROFILE is not set" \
  /bin/bash "$scripts/notarize.sh" "$work/Inkwell.dmg"
run "notarising anything but an app or a dmg" "only an .app or a .dmg" \
  NOTARY_PROFILE=x /bin/bash "$scripts/notarize.sh" "$work/some.zip"
run "verifying a dmg that is not there" "no dmg at" \
  /bin/bash "$scripts/verify-release.sh" "$work/missing.dmg" --notarized
run "an appcast without Sparkle's tools" "SPARKLE_BIN has no generate_appcast" \
  /bin/bash "$scripts/appcast.sh" --dmg "$work/Inkwell.dmg" --version 1.2.3 \
  --download-prefix https://example.invalid/v1.2.3/ --out "$work/out"
run "an appcast pointing at plain http" "must be an https URL ending in /" \
  /bin/bash "$scripts/appcast.sh" --dmg "$work/Inkwell.dmg" --version 1.2.3 \
  --download-prefix http://example.invalid/v1.2.3/ --out "$work/out"

finish
