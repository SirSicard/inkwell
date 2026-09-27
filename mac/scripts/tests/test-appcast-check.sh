#!/usr/bin/env bash
# appcast-check.swift on synthetic feeds: what it must pass, and every way it must fail. The feeds
# are signed here the way Sparkle's generate_appcast signs them (Ed25519 over the archive; over the
# feed's bytes before its trailing signature comment), with a throwaway key. Offline; needs swiftc.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/assert.sh"

work="$(mktemp -d "${TMPDIR:-/tmp}/ink-appcast-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT

# Compiled once: each `swift file.swift` run would compile it again.
swiftc -O -o "$work/check" "$here/../appcast-check.swift" 2>"$work/build.log" \
  || { cat "$work/build.log"; exit 1; }

# The test's signer: sign <private-key-file> <file> prints the Ed25519 signature, base64.
cat >"$work/sign.swift" <<'SWIFT'
import CryptoKit
import Foundation
let seed = try String(contentsOfFile: CommandLine.arguments[1], encoding: .utf8)
let key = try Curve25519.Signing.PrivateKey(rawRepresentation: Data(base64Encoded: seed)!)
let data = try Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[2]))
print(try key.signature(for: data).base64EncodedString())
SWIFT
swiftc -O -o "$work/sign" "$work/sign.swift" 2>"$work/build.log" || { cat "$work/build.log"; exit 1; }

key="$("$work/check" throwaway-key "$work/key")"
other="$("$work/check" throwaway-key "$work/other.key")"
assert_contains "a throwaway key's private half is readable by its owner only" \
  "$(stat -f %Lp "$work/key")" "600"
assert_status "a public key is 32 bytes of base64" 44 "${#key}"

head -c 4096 /dev/urandom >"$work/Inkwell.dmg"
head -c 4096 /dev/urandom >"$work/Other.dmg"
url="https://example.invalid/download/v1.2.3/Inkwell.dmg"
size="$(stat -f %z "$work/Inkwell.dmg")"

# feed <out> <item signature attribute or ""> [version] [second item]: a feed signed with key.
feed() {
  local out="$1" item_signature="$2" version="${3:-1.2.3}" extra="${4:-}"
  local attribute=""
  [ -n "$item_signature" ] && attribute=" sparkle:edSignature=\"$item_signature\""
  cat >"$work/body.xml" <<XML
<?xml version="1.0" standalone="yes"?>
<rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle" version="2.0">
    <channel>
        <title>Inkwell</title>
        <item>
            <title>$version</title>
            <sparkle:version>$version</sparkle:version>
            <enclosure url="$url" length="$size" type="application/octet-stream"$attribute/>
        </item>$extra
    </channel>
</rss>
XML
  local signature length
  signature="$("$work/sign" "$work/key" "$work/body.xml")"
  length="$(stat -f %z "$work/body.xml")"
  { cat "$work/body.xml"; printf '<!-- sparkle-signatures:\nedSignature: %s\nlength: %s\n-->\n' "$signature" "$length"; } >"$out"
}

# run <label> <expected status> <expected text> <check arguments...>
run() {
  local label=$1 want=$2 text=$3 out status=0
  shift 3
  out="$("$work/check" "$@" 2>&1)" || status=$?
  assert_status "$label" "$want" "$status"
  assert_contains "$label: says why" "$out" "$text"
}

item_sig="$("$work/sign" "$work/key" "$work/Inkwell.dmg")"
feed "$work/good.xml" "$item_sig"
run "a signed feed and a signed item" 0 "its signature verifies against the app's key" \
  verify "$work/good.xml" "$work/Inkwell.dmg" 1.2.3 "$url" "$key"
run "the feed alone" 0 "feed: signed" feed "$work/good.xml" "$key"
run "one signature over a file" 0 "verifies" signature "$work/Inkwell.dmg" "$item_sig" "$key"

run "another key than the app's" 1 "the feed's signature does not verify" \
  verify "$work/good.xml" "$work/Inkwell.dmg" 1.2.3 "$url" "$other"
sed 's#<title>1.2.3</title>#<title>1.2.4</title>#' "$work/good.xml" >"$work/tampered.xml"
run "a feed changed after signing" 1 "the feed's signature does not verify" \
  verify "$work/tampered.xml" "$work/Inkwell.dmg" 1.2.3 "$url" "$key"
run "... also on its own" 1 "does not verify" feed "$work/tampered.xml" "$key"
sed 's#^<!-- sparkle-signatures:#<!-- slipped in --><!-- sparkle-signatures:#' "$work/good.xml" >"$work/gap.xml"
run "bytes between the signed part and its signature" 1 "but its comment starts at" \
  verify "$work/gap.xml" "$work/Inkwell.dmg" 1.2.3 "$url" "$key"
sed '/^<!-- sparkle-signatures:/,$d' "$work/good.xml" >"$work/unsigned-feed.xml"
run "an unsigned feed" 1 "the feed is not signed" \
  verify "$work/unsigned-feed.xml" "$work/Inkwell.dmg" 1.2.3 "$url" "$key"
run "another download URL" 1 "not https://example.invalid/other.dmg" \
  verify "$work/good.xml" "$work/Inkwell.dmg" 1.2.3 "https://example.invalid/other.dmg" "$key"
run "another version" 1 "0 items for version 1.2.4" \
  verify "$work/good.xml" "$work/Inkwell.dmg" 1.2.4 "$url" "$key"
run "another archive of the same size" 1 "the item's signature does not verify" \
  verify "$work/good.xml" "$work/Other.dmg" 1.2.3 "$url" "$key"
head -c 100 /dev/urandom >"$work/Short.dmg"
run "an archive of another size" 1 "the item's length is not the dmg's size" \
  verify "$work/good.xml" "$work/Short.dmg" 1.2.3 "$url" "$key"
run "a signature made by another key" 1 "does not verify" \
  signature "$work/Inkwell.dmg" "$item_sig" "$other"
run "a key that is not 32 bytes" 1 "not 32 bytes of base64" \
  verify "$work/good.xml" "$work/Inkwell.dmg" 1.2.3 "$url" "c2hvcnQ="

feed "$work/unsigned-item.xml" ""
run "an unsigned item" 1 "has no EdDSA signature" \
  verify "$work/unsigned-item.xml" "$work/Inkwell.dmg" 1.2.3 "$url" "$key"
run "... allowed in the rehearsal" 0 "not signed (the app carries no update key)" \
  verify "$work/unsigned-item.xml" "$work/Inkwell.dmg" 1.2.3 "$url" "$key" --allow-unsigned-item
run "an unknown flag" 1 "unknown argument" \
  verify "$work/unsigned-item.xml" "$work/Inkwell.dmg" 1.2.3 "$url" "$key" --lenient

duplicate="
        <item><sparkle:version>1.2.3</sparkle:version><enclosure url=\"$url\" length=\"$size\"/></item>"
feed "$work/duplicate.xml" "$item_sig" 1.2.3 "$duplicate"
run "two items for one version" 1 "2 items for version 1.2.3" \
  verify "$work/duplicate.xml" "$work/Inkwell.dmg" 1.2.3 "$url" "$key"

finish
