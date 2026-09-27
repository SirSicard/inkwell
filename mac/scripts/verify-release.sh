#!/usr/bin/env bash
# Asks Gatekeeper about a release dmg the way a user's Mac will: a quarantined copy, as a browser
# leaves a download, then the dmg (open) and the app inside it (execute), the stapled tickets, and
# what the app's Info.plist promises. A green notarisation submission is not a passing verdict:
# that exact gap once shipped a dmg Gatekeeper rejected (docs/RELEASING.md).
#
#   mac/scripts/verify-release.sh <dmg> [--notarized] [--version X.Y.Z] [--update-key]
#
#   --notarized    require both verdicts to be "accepted" from "Notarized Developer ID", and both
#                  tickets stapled (the release). Without it the verdicts are only reported: before
#                  notarisation both are "rejected", "source=Unnotarized Developer ID", and only a
#                  signature that does not verify fails.
#   --version      require the app's CFBundleShortVersionString and CFBundleVersion to be it
#   --update-key   require SUPublicEDKey to be a key (32 bytes of base64), not the empty
#                  placeholder: a release published without one could never update itself
#
# Everything printed is redacted (lib/redact-signing.sh): a Developer ID names a person and a team.
set -euo pipefail

mac="$(cd "$(dirname "$0")/.." && pwd)"
. "$mac/scripts/lib/redact-signing.sh"
. "$mac/scripts/lib/dmg.sh"
fail() { echo "verify-release: FAIL $*" >&2; exit 1; }

[ $# -ge 1 ] || fail "usage: verify-release.sh <dmg> [--notarized] [--version X.Y.Z] [--update-key]"
dmg="$1"
shift
notarized=0
version=""
update_key=0
while [ $# -gt 0 ]; do
  case "$1" in
    --notarized) notarized=1 ;;
    --update-key) update_key=1 ;;
    --version)
      [ $# -ge 2 ] || fail "--version needs a version"
      version="$2"
      shift
      ;;
    *) fail "unknown argument: $1" ;;
  esac
  shift
done
[ -f "$dmg" ] || fail "no dmg at $dmg"

work="$(mktemp -d "${TMPDIR:-/tmp}/ink-verify.XXXXXX")"
trap 'dmg_detach "$work/mnt" || true; rm -rf "$work"' EXIT

codesign --verify --strict "$dmg" 2>&1 | redact_signing || fail "the dmg's signature does not verify"

# The copy a browser leaves: the quarantine flag is what makes Gatekeeper assess it at all.
cp "$dmg" "$work/download.dmg"
xattr -w com.apple.quarantine "0081;$(printf %x "$(date +%s)");Safari;" "$work/download.dmg"

# verdict <label> <spctl arguments...>: prints the redacted verdict; returns 0 only when it is
# "accepted" from "Notarized Developer ID". spctl's own status is not the verdict's whole story.
verdict() {
  local label="$1" out
  shift
  out="$(spctl "$@" 2>&1)" || true
  printf '%s:\n' "$label"
  redact_signing <<<"$out" | sed "s#$work/##; s/^/  /"
  grep -q ': accepted$' <<<"$out" && grep -q '^source=Notarized Developer ID$' <<<"$out"
}

dmg_ok=0
verdict "the dmg (spctl -a -vv -t open)" -a -vv -t open --context context:primary-signature "$work/download.dmg" \
  && dmg_ok=1

dmg_attach "$work/download.dmg" "$work/mnt" || fail "the dmg does not mount"
app="$work/mnt/Inkwell.app"
[ -d "$app" ] || fail "the dmg holds no Inkwell.app"
[ "$(readlink "$work/mnt/Applications")" = /Applications ] || fail "the dmg has no link to /Applications"
codesign --verify --deep --strict "$app" 2>&1 | redact_signing || fail "the app's signature does not verify"
app_ok=0
verdict "the app inside (spctl -a -vv -t execute)" -a -vv -t execute "$app" && app_ok=1

if [ "$notarized" = 1 ]; then
  [ "$dmg_ok" = 1 ] || fail "Gatekeeper does not accept the dmg as notarized"
  [ "$app_ok" = 1 ] || fail "Gatekeeper does not accept the app as notarized"
  # Stapled, so both checks also pass offline.
  xcrun stapler validate -q "$dmg" || fail "the dmg has no valid stapled ticket"
  xcrun stapler validate -q "$app" || fail "the app has no valid stapled ticket"
  echo "notarized: both accepted, both tickets stapled"
fi

plist="$app/Contents/Info.plist"
read_key() { /usr/libexec/PlistBuddy -c "Print :$1" "$plist" 2>/dev/null || true; }
[ "$(read_key CFBundleIdentifier)" = com.inkwell.app ] || fail "the app is not com.inkwell.app"
if [ -n "$version" ]; then
  [ "$(read_key CFBundleShortVersionString)" = "$version" ] || fail "the app's version is not $version"
  [ "$(read_key CFBundleVersion)" = "$version" ] || fail "the app's build number is not $version"
  echo "version: $version"
fi
key="$(read_key SUPublicEDKey)"
key_bytes="$( (base64 -D <<<"$key" 2>/dev/null || true) | wc -c | tr -d ' ')"
if [ -n "$key" ] && [ "$key_bytes" = 32 ]; then
  echo "update key: set"
elif [ "$update_key" = 1 ]; then
  fail "the app carries no update key (SUPublicEDKey): it could never update itself"
else
  echo "update key: not set, so this build never checks for updates (a tag would stop here)"
fi
echo "verified: $dmg"
