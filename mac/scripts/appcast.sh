#!/usr/bin/env bash
# Signs the Sparkle appcast for a release: Sparkle's generate_appcast over the new dmg (and over
# the feed already published, so earlier items stay), then appcast-check.swift reads the result
# the way an installed Inkwell will, against the key the app in the dmg carries.
#
#   mac/scripts/appcast.sh --dmg <dmg> --version <X.Y.Z> --download-prefix <url/> --out <dir> \
#       [--previous <appcast.xml>] [--link <url>] [--rehearsal]  < private-key
#
# Environment:
#   SPARKLE_BIN  the directory holding Sparkle's generate_appcast and sign_update (the release's
#                tools tarball, checked against its hash before this runs)
#
# The EdDSA private key arrives on stdin, in the form Sparkle's generate_keys exports it, and goes
# only to Sparkle's tools, through a pipe: it is never written to disk or printed. Writes
# <dir>/appcast.xml.
#
# The check key is the app's own SUPublicEDKey, so a feed signed with any other key fails here,
# not on users' Macs; a previous feed must verify against it too before its items are carried
# over (generate_appcast re-signs the whole feed, so an unchecked one would be laundered).
#
# --rehearsal (the dry run): stdin is not read. A throwaway key signs, the check uses it, and the
# previous feed is left out. An app without a key yet has no item signature to check, so the dmg
# is signed directly with the throwaway key and that signature is checked instead: every tool the
# release uses still runs.
set -euo pipefail

mac="$(cd "$(dirname "$0")/.." && pwd)"
. "$mac/scripts/lib/dmg.sh"
fail() { echo "appcast: $*" >&2; exit 1; }

dmg="" version="" prefix="" out="" previous="" link="" rehearsal=0
while [ $# -gt 0 ]; do
  case "$1" in
    --dmg | --version | --download-prefix | --out | --previous | --link)
      [ $# -ge 2 ] || fail "$1 needs a value"
      case "$1" in
        --dmg) dmg="$2" ;;
        --version) version="$2" ;;
        --download-prefix) prefix="$2" ;;
        --out) out="$2" ;;
        --previous) previous="$2" ;;
        --link) link="$2" ;;
      esac
      shift
      ;;
    --rehearsal) rehearsal=1 ;;
    *) fail "unknown argument: $1" ;;
  esac
  shift
done
[ -f "$dmg" ] || fail "no dmg at '$dmg'"
[ -n "$version" ] || fail "--version is required"
case "$prefix" in https://*/) ;; *) fail "--download-prefix must be an https URL ending in /" ;; esac
[ -n "$out" ] || fail "--out is required"
for tool in generate_appcast sign_update; do
  [ -x "${SPARKLE_BIN:-}/$tool" ] || fail "SPARKLE_BIN has no $tool"
done
check=(swift "$mac/scripts/appcast-check.swift")

work="$(mktemp -d "${TMPDIR:-/tmp}/ink-appcast.XXXXXX")"
trap 'dmg_detach "$work/mnt" || true; rm -rf "$work"' EXIT

# The key and version the app in the dmg carries.
dmg_attach "$dmg" "$work/mnt" || fail "the dmg does not mount"
plist="$work/mnt/Inkwell.app/Contents/Info.plist"
[ -f "$plist" ] || fail "the dmg holds no Inkwell.app"
app_key="$(/usr/libexec/PlistBuddy -c 'Print :SUPublicEDKey' "$plist" 2>/dev/null || true)"
app_version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$plist" 2>/dev/null || true)"
dmg_detach "$work/mnt"
[ "$app_version" = "$version" ] || fail "the app in the dmg is build $app_version, not $version"

mkdir -p "$work/archives"
cp "$dmg" "$work/archives/"
if [ "$rehearsal" = 1 ]; then
  key="$work/throwaway.key"
  check_key="$("${check[@]}" throwaway-key "$key")"
  echo "rehearsal: a throwaway key signs; nothing here is published"
  [ -n "$previous" ] && echo "rehearsal: the previous feed is left out"
  sign_with() { "$@" --ed-key-file "$key"; }
else
  [ -n "$app_key" ] || fail "the app carries no update key (SUPublicEDKey): generate it first (docs/RELEASING.md)"
  check_key="$app_key"
  private_key="$(cat)"
  [ -n "$private_key" ] || fail "no private key on stdin"
  sign_with() { printf '%s\n' "$private_key" | "$@" --ed-key-file -; }
  if [ -n "$previous" ] && [ -s "$previous" ]; then
    "${check[@]}" feed "$previous" "$check_key" \
      || fail "the published feed does not verify against the app's key: not carried over"
    cp "$previous" "$work/archives/appcast.xml"
    echo "previous feed: verified, its items carried over"
  fi
fi

args=(--download-url-prefix "$prefix" --maximum-deltas 0)
[ -n "$link" ] && args+=(--link "$link")
sign_with "$SPARKLE_BIN/generate_appcast" "${args[@]}" "$work/archives" 2>&1 | tee "$work/generate.log"
# In a release the app's key must be the signing key; Sparkle only warns when it is not.
if [ "$rehearsal" = 0 ] && grep -qi 'does not match' "$work/generate.log"; then
  fail "generate_appcast signed with a key that is not the app's"
fi

url="$prefix$(basename "$dmg")"
if [ "$rehearsal" = 1 ] && [ -z "$app_key" ]; then
  "${check[@]}" verify "$work/archives/appcast.xml" "$dmg" "$version" "$url" "$check_key" --allow-unsigned-item
  signature="$(sign_with "$SPARKLE_BIN/sign_update" -p "$dmg")"
  "${check[@]}" signature "$dmg" "$signature" "$check_key"
else
  "${check[@]}" verify "$work/archives/appcast.xml" "$dmg" "$version" "$url" "$check_key"
fi

mkdir -p "$out"
cp "$work/archives/appcast.xml" "$out/appcast.xml"
echo "appcast: $out/appcast.xml"
