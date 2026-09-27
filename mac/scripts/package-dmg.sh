#!/usr/bin/env bash
# Wraps a built, signed Inkwell.app in the dmg people download: the app and a link to
# /Applications, compressed, and the dmg itself signed with the same identity.
#
#   mac/scripts/package-dmg.sh <Inkwell.app> <out.dmg> [--timestamp]
#
# Environment:
#   INK_SIGN_IDENTITY  the identity the app was signed with (as build-mac.sh takes it). Required:
#                      an ad-hoc dmg is not a download anyone should be offered. Never printed.
#
# --timestamp signs with Apple's secure timestamp, which notarisation requires (the release). The
# app should already be notarised and stapled (notarize.sh) before it is packaged: the dmg's own
# ticket does not travel with an app copied out of it, and the app's does.
set -euo pipefail

mac="$(cd "$(dirname "$0")/.." && pwd)"
. "$mac/scripts/lib/redact-signing.sh"
fail() { echo "package-dmg: $*" >&2; exit 1; }

[ $# -ge 2 ] || fail "usage: package-dmg.sh <Inkwell.app> <out.dmg> [--timestamp]"
app="$1"
out="$2"
shift 2
stamp=(--timestamp=none)
for arg in "$@"; do
  case "$arg" in
    --timestamp) stamp=(--timestamp) ;;
    *) fail "unknown argument: $arg" ;;
  esac
done
identity="${INK_SIGN_IDENTITY:-}"
[ -n "$identity" ] || fail "INK_SIGN_IDENTITY is not set: the dmg would go out unsigned"
[ -d "$app/Contents" ] || fail "no app bundle at $app"
case "$out" in *.dmg) ;; *) fail "the output must end in .dmg: $out" ;; esac
codesign --verify --deep --strict "$app" 2>&1 | redact_signing "$identity" \
  || fail "the app's signature does not verify: build it with build-mac.sh first"

work="$(mktemp -d "${TMPDIR:-/tmp}/ink-dmg.XXXXXX")"
trap 'rm -rf "$work"' EXIT
# ditto keeps the bundle exactly as signed (symlinks, extended attributes, the stapled ticket).
ditto "$app" "$work/root/Inkwell.app"
ln -s /Applications "$work/root/Applications"

rm -f "$out"
# HFS+ and zlib (UDZO): readable by every macOS the app supports, and by Sparkle, which mounts
# the dmg to install an update from it.
hdiutil create -quiet -volname Inkwell -srcfolder "$work/root" -fs HFS+ -format UDZO -ov "$out"
codesign --force "${stamp[@]}" --sign "$identity" "$out" 2>&1 | redact_signing "$identity"

codesign --verify --strict "$out" 2>&1 | redact_signing "$identity" || fail "the dmg's signature does not verify"
hdiutil verify -quiet "$out" || fail "the dmg does not verify"
echo "packaged: $out ($(stat -f %z "$out") bytes, signed)"
