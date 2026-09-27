#!/usr/bin/env bash
# build-mac.sh prints signing details only through redact_signing: no certificate name (a
# person's legal name, for an individual developer account) and no team ID may survive it.
# The name and team ID below are made up.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/assert.sh"
. "$here/../lib/redact-signing.sh"

name="Jane Q. Example"
team="Q7X2K9M4P1"
hash="0123456789ABCDEF0123456789ABCDEF01234567"

# Redacts $2 into $out (not a subshell: the assertions must count here).
check() { # label, input, [literal identity]
  out="$(printf '%s\n' "$2" | redact_signing "${3:-}")"
  assert_absent "$1: the name" "$out" "$name"
  assert_absent "$1: the name's surname" "$out" "Example"
  assert_absent "$1: the team ID" "$out" "$team"
}

# codesign -d -r- of a Developer ID app, the form macOS prints today.
check "requirement, OU unquoted" \
  "designated => identifier \"com.inkwell.app\" and anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] /* exists */ and certificate leaf[field.1.2.840.113635.100.6.1.13] /* exists */ and certificate leaf[subject.OU] = $team"
assert_contains "requirement: the structure survives" "$out" 'identifier "com.inkwell.app" and anchor apple generic'
assert_contains "requirement: the OID survives" "$out" "field.1.2.840.113635.100.6.2.6"

# A requirement that names the certificate itself (an explicit requirement, or an older signer).
check "requirement, CN and OU quoted" \
  "designated => identifier \"com.inkwell.app\" and anchor apple generic and certificate leaf[subject.CN] = \"Developer ID Application: $name ($team)\" and certificate leaf[subject.OU] = \"$team\""
assert_contains "CN: replaced, not dropped" "$out" 'subject.CN] = "<certificate name>"'

check "requirement, organisation" \
  "certificate leaf[subject.O] = \"$name\" and certificate leaf[subject.OU] = $team"
assert_contains "O: replaced" "$out" "subject.O] = <name>"

# codesign -dv, and the errors codesign prints for an identity.
check "codesign -dv" "Authority=Developer ID Application: $name ($team)
Authority=Developer ID Certification Authority
TeamIdentifier=$team"
check "codesign error by name" "Developer ID Application: $name ($team): no identity found"
check "a development certificate" "Apple Development: $name ($team)"

# The identity as given in INK_SIGN_IDENTITY (a SHA-1 hash or a name), literally.
out="$(printf '%s\n' "error: $hash: no identity found" | redact_signing "$hash")"
assert_absent "the identity hash" "$out" "$hash"
out="$(printf '%s\n' "error: $name: ambiguous" | redact_signing "$name")"
assert_absent "an identity given by a bare name" "$out" "$name"

# An ad-hoc requirement is a hash of the code, not of anyone: it stays readable.
out="$(printf '%s\n' '# designated => cdhash H"af5beb612997d74cdada1685b24434c8257d8b8b"' | redact_signing "")"
assert_contains "an ad-hoc cdhash survives" "$out" 'cdhash H"af5beb612997d74cdada1685b24434c8257d8b8b"'

finish
