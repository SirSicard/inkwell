#!/usr/bin/env bash
# Notarises Inkwell.app or its dmg with Apple, waits for the verdict, and staples the ticket.
#
#   mac/scripts/notarize.sh <Inkwell.app | file.dmg>
#
# Environment:
#   NOTARY_PROFILE   a notarytool keychain profile (`xcrun notarytool store-credentials`); the
#                    release workflow stores the APPLE_* secrets under one in its throwaway keychain
#   NOTARY_KEYCHAIN  the keychain holding that profile (optional; default: the search list)
#
# An app is sent as a zip (the service takes no bare bundle) and the ticket is stapled to the app
# itself, so it travels into the dmg and onto the user's disk. Notarise the app before packaging
# it, then the dmg.
#
# The service is a queue whose wait is unbounded in practice, and polling it has died on a
# transient network error after 51 minutes (the 0.2 releases). So the upload happens once and
# only the wait is retried: the submission keeps its place in the queue. Anything but "Accepted"
# fetches the service's log, redacted, and fails.
set -euo pipefail

mac="$(cd "$(dirname "$0")/.." && pwd)"
. "$mac/scripts/lib/redact-signing.sh"
fail() { echo "notarize: $*" >&2; exit 1; }

[ $# -eq 1 ] || fail "usage: notarize.sh <Inkwell.app | file.dmg>"
target="${1%/}"
[ -n "${NOTARY_PROFILE:-}" ] || fail "NOTARY_PROFILE is not set (a notarytool keychain profile)"
auth=(--keychain-profile "$NOTARY_PROFILE")
[ -n "${NOTARY_KEYCHAIN:-}" ] && auth+=(--keychain "$NOTARY_KEYCHAIN")

work="$(mktemp -d "${TMPDIR:-/tmp}/ink-notary.XXXXXX")"
trap 'rm -rf "$work"' EXIT

case "$target" in
  *.app)
    [ -d "$target/Contents" ] || fail "no app bundle at $target"
    upload="$work/$(basename "$target" .app).zip"
    # ditto keeps the bundle's symlinks and signatures; --keepParent puts the .app in the zip.
    ditto -c -k --keepParent "$target" "$upload"
    ;;
  *.dmg)
    [ -f "$target" ] || fail "no dmg at $target"
    upload="$target"
    ;;
  *) fail "only an .app or a .dmg can be notarised: $target" ;;
esac

# field <json file> <key>: a top-level string from notarytool's JSON (plutil reads JSON).
field() { plutil -extract "$2" raw -o - "$1" 2>/dev/null || true; }

xcrun notarytool submit "$upload" "${auth[@]}" --no-wait --output-format json >"$work/submit.json" 2>"$work/submit.err" \
  || { redact_signing <"$work/submit.err" >&2; fail "the upload failed"; }
id="$(field "$work/submit.json" id)"
[ -n "$id" ] || { redact_signing <"$work/submit.json" >&2; fail "the service returned no submission id"; }
echo "submitted $(basename "$upload"): $id"

status=""
for attempt in 1 2 3; do
  # wait's own exit status is not the verdict (it also ends on a timeout): info is read after it.
  xcrun notarytool wait "$id" "${auth[@]}" --timeout 2h >/dev/null 2>"$work/wait.err" \
    || echo "notarize: waiting ended early (attempt $attempt): $(redact_signing <"$work/wait.err" | tail -1)" >&2
  if xcrun notarytool info "$id" "${auth[@]}" --output-format json >"$work/info.json" 2>/dev/null; then
    status="$(field "$work/info.json" status)"
    [ "$status" = "In Progress" ] || break
  fi
  sleep 30
done
echo "verdict: ${status:-unknown}"
if [ "$status" != Accepted ]; then
  # The log lists every problem by file; it names the certificate, so it is redacted.
  xcrun notarytool log "$id" "${auth[@]}" 2>&1 | redact_signing >&2 || true
  fail "$(basename "$upload") was not accepted (${status:-no verdict}); the log is above"
fi

xcrun stapler staple -q "$target" || fail "the ticket could not be stapled to $target"
xcrun stapler validate -q "$target" || fail "the stapled ticket on $target does not validate"
echo "notarized and stapled: $target"
