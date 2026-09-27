#!/usr/bin/env bash
# Builds the Mac app, mac/build/Inkwell.app: the core (build-core.sh), the Swift package, the
# bundle around them (mac/Info.plist), and its signature (mac/Inkwell.entitlements). No
# .xcodeproj: SwiftPM builds, this script bundles and signs.
#
#   mac/scripts/build-mac.sh                    release build
#   mac/scripts/build-mac.sh --debug            debug build
#   mac/scripts/build-mac.sh --skip-core        reuse mac/build/InkCore.xcframework as it is
#   mac/scripts/build-mac.sh --same-requirement-as /Applications/Inkwell.app
#                                               and fail unless the new app's designated
#                                               requirement is that app's (grants carry over)
#
# Environment:
#   INK_SIGN_IDENTITY  the signing identity: a SHA-1 hash or a name from
#                      `security find-identity -v -p codesigning`. Unset: ad-hoc, which is for
#                      build checks (CI) only, never for an app anyone grants permissions to.
#                      The identity is never printed, and never belongs in this repository.
#   INK_VERSION, INK_BUILD_NUMBER   the bundle's version (default 1.0.0 and 1)
#   INK_CORE_FEATURES, INK_CORE_PROFILE, CARGO_FLAGS   passed to build-core.sh
#
# Why the signing steps are as they are:
# - TCC keys every grant (microphone, system audio, Accessibility) to the app's designated
#   requirement. An ad-hoc signature's requirement is its own hash, so to TCC each ad-hoc
#   rebuild is a new app: grants reset, and a far end that silently lost its grant records
#   nothing for weeks. A stable identity keeps one requirement across builds.
# - The hardened runtime blocks the microphone unless the audio-input entitlement is signed in,
#   and it blocks it silently: no prompt, just a deny.
# - Signing the bundle does not sign a loose executable elsewhere in it (only nested bundles are
#   walked), and `codesign --verify --deep --strict` still passes with that file ad-hoc. So every
#   Mach-O is signed on its own, inside out, and each one's signature is checked afterwards.
# - A verification that cannot fail the build is not one: every check below exits non-zero.
set -euo pipefail

mac="$(cd "$(dirname "$0")/.." && pwd)"
config=release
skip_core=0
same_as=""
while [ $# -gt 0 ]; do
  case "$1" in
    --debug) config=debug ;;
    --release) config=release ;;
    --skip-core) skip_core=1 ;;
    --same-requirement-as)
      [ $# -ge 2 ] || { echo "--same-requirement-as needs an app path" >&2; exit 2; }
      same_as="$2"
      shift
      ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

bundle_id=com.inkwell.app
app="$mac/build/Inkwell.app"
identity="${INK_SIGN_IDENTITY:-}"
fail() { echo "build-mac: $*" >&2; exit 1; }

if [ -n "$identity" ]; then
  # Checked before building, so a typo fails in seconds. Listing identities reads certificates,
  # not private keys: it never shows a keychain prompt. (The codesign calls below may.)
  security find-identity -v -p codesigning | grep -qF -- "$identity" \
    || fail "INK_SIGN_IDENTITY names no valid code-signing identity in the keychain"
else
  echo "build-mac: INK_SIGN_IDENTITY is not set: signing ad-hoc. Fine for a build check; do not" >&2
  echo "           grant this build any permission (TCC forgets it at the next build)." >&2
fi

# --- build --------------------------------------------------------------------------------------
if [ "$skip_core" = 1 ]; then
  [ -d "$mac/build/InkCore.xcframework" ] || fail "--skip-core, but mac/build/InkCore.xcframework is missing"
else
  "$mac/scripts/build-core.sh"
fi
swift build --package-path "$mac" -c "$config" --product Inkwell
bin="$(swift build --package-path "$mac" -c "$config" --show-bin-path)"

# --- bundle -------------------------------------------------------------------------------------
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin/Inkwell" "$app/Contents/MacOS/Inkwell"
cp "$mac/Info.plist" "$app/Contents/Info.plist"
plutil -replace CFBundleShortVersionString -string "${INK_VERSION:-1.0.0}" "$app/Contents/Info.plist"
plutil -replace CFBundleVersion -string "${INK_BUILD_NUMBER:-1}" "$app/Contents/Info.plist"
printf 'APPL????' >"$app/Contents/PkgInfo"
# SwiftPM resource bundles (a dependency's data files). Bundle.module looks in
# Bundle.main.resourceURL first, which is Contents/Resources in an app.
find "$bin" -maxdepth 1 -name '*.bundle' -type d -exec cp -R {} "$app/Contents/Resources/" \;

# --- sign ---------------------------------------------------------------------------------------
sign_as=("${identity:--}")
# --timestamp=none: a local build does not ask Apple's timestamp server. The release pipeline
# signs with a secure timestamp, which notarisation requires.
sign() { codesign --force --options runtime --timestamp=none --sign "${sign_as[0]}" "$@"; }

is_macho() { file -b "$1" | grep -q '^Mach-O'; }

# Inside out: resource bundles, then every loose Mach-O other than the main executable, then the
# app itself with its entitlements. A Mach-O inside a nested bundle has to be signed as part of
# that bundle; nothing produces one yet, so it stops the build rather than going out half signed.
nested=()
while IFS= read -r -d '' f; do
  # Matched on the path inside the bundle: the bundle's own path ends in .app too.
  case "${f#"$app/Contents/"}" in
    MacOS/Inkwell) continue ;;
    *.bundle/* | *.framework/* | *.app/* | *.appex/* | *.xpc/*)
      is_macho "$f" && fail "executable code inside a nested bundle is not signed by this script yet: ${f#"$app/"}"
      continue
      ;;
  esac
  is_macho "$f" && nested+=("$f")
done < <(find "$app/Contents" -type f -print0)
for b in "$app/Contents/Resources/"*.bundle; do
  [ -d "$b" ] && sign "$b"
done
for f in ${nested[@]+"${nested[@]}"}; do
  sign "$f"
done
sign --entitlements "$mac/Inkwell.entitlements" "$app"

# --- verify -------------------------------------------------------------------------------------
codesign --verify --deep --strict "$app" || fail "the signature does not verify"

info="$(codesign -dv "$app" 2>&1)"
grep -q "^Identifier=$bundle_id\$" <<<"$info" || fail "the app is not signed as $bundle_id"
grep -Eq '^CodeDirectory .*flags=0x[0-9a-f]+\([^)]*runtime' <<<"$info" || fail "the hardened runtime is off"

signed_ents="$mac/build/signed-entitlements.plist"
codesign -d --entitlements - --xml "$app" >"$signed_ents" 2>/dev/null || fail "no entitlements could be read back"
# PlistBuddy, not plutil: plutil's key paths split on the dots in the key.
[ "$(/usr/libexec/PlistBuddy -c 'Print :com.apple.security.device.audio-input' "$signed_ents" 2>/dev/null)" = true ] \
  || fail "the audio-input entitlement is missing: the hardened runtime would deny the microphone silently"

# Each Mach-O on its own: same kind of signature as the app (ad-hoc or not), same team.
team_of() { codesign -dv "$1" 2>&1 | sed -n 's/^TeamIdentifier=//p'; }
app_team="$(team_of "$app")"
for f in "$app/Contents/MacOS/Inkwell" ${nested[@]+"${nested[@]}"}; do
  details="$(codesign -dv "$f" 2>&1)" || fail "${f#"$app/"} is not signed"
  if [ -n "$identity" ]; then
    grep -q 'Signature=adhoc' <<<"$details" && fail "${f#"$app/"} is signed ad-hoc"
    [ "$(team_of "$f")" = "$app_team" ] || fail "${f#"$app/"} is signed by another team than the app"
  fi
  grep -Eq 'flags=0x[0-9a-f]+\([^)]*runtime' <<<"$details" || fail "${f#"$app/"} lacks the hardened runtime"
done

if [ -n "$same_as" ]; then
  # Compared, never printed in full: a Developer ID requirement names the team.
  # An ad-hoc signature's requirement is implicit, printed as "# designated => cdhash ...".
  requirement() { codesign -d -r- "$1" 2>&1 | sed -nE 's/^(# )?designated => //p'; }
  ours="$(requirement "$app")"
  theirs="$(requirement "$same_as")"
  [ -n "$theirs" ] || fail "no designated requirement read from $same_as"
  if [ "$ours" != "$theirs" ]; then
    mask() { sed -E 's/(subject\.OU\] = )"?[A-Z0-9]{10}"?/\1<team>/'; }
    echo "  this build: $(mask <<<"$ours")" >&2
    echo "  $same_as: $(mask <<<"$theirs")" >&2
    fail "the designated requirement differs from $same_as: TCC grants will not carry over"
  fi
  echo "designated requirement: the same as $same_as"
fi

if [ -n "$identity" ]; then
  echo "signed: INK_SIGN_IDENTITY, hardened runtime, audio-input; ${#nested[@]} loose tool(s) signed"
else
  echo "signed: ad-hoc (build check only), hardened runtime, audio-input"
fi
echo "built: $app ($config)"
