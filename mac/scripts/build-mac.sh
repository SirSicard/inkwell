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
#   mac/scripts/build-mac.sh --timestamp        sign with Apple's secure timestamp, which
#                                               notarisation requires (the release; needs
#                                               INK_SIGN_IDENTITY, --engines and the network)
#   mac/scripts/build-mac.sh --engines          the core with its speech engines, as released:
#                                               llama.cpp (Qwen3-ASR), Silero VAD and
#                                               NeMo-Speech.cpp (the diarizer; needs
#                                               NEMO_SPEECH_DIR), whose libraries are bundled in
#                                               Contents/Frameworks
#
# Environment:
#   INK_SIGN_IDENTITY  the signing identity: a SHA-1 hash or a name from
#                      `security find-identity -v -p codesigning`. Unset: ad-hoc, which is for
#                      build checks (CI) only, never for an app anyone grants permissions to.
#                      The identity is never printed, and never belongs in this repository.
#   INK_VERSION, INK_BUILD_NUMBER   the bundle's version (default 1.0.0 and 1)
#   INK_CORE_FEATURES, INK_CORE_PROFILE, CARGO_FLAGS   passed to build-core.sh (--engines sets
#                      the features itself)
#   NEMO_SPEECH_DIR    with --engines: the NeMo-Speech.cpp prefix that
#                      core/crates/ink-engines/native/build-nemo-speech.sh installed
#   MACOSX_DEPLOYMENT_TARGET   the oldest macOS the app runs on (default 26.0, Package.swift's)
#   INK_ALLOW_NEWER_MACOS=1    a local build only: code built for a newer macOS than that (a
#                      Homebrew built for this Mac's own macOS, copied into the NeMo prefix) is a
#                      warning instead of a failure. Refused with --timestamp.
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
# - Frameworks (Sparkle) are re-signed with the app's identity, their own helpers first, because
#   the hardened runtime's library validation loads only code signed by the app's team (or
#   Apple), and notarisation accepts only code signed with the Developer ID. Only the app has
#   entitlements; every other signature is checked to carry none.
# - The engines' libraries (NeMo-Speech.cpp, its ggml, SentencePiece and Abseil) are plain dylibs
#   the executable loads through its rpath, so they go in Contents/Frameworks beside Sparkle and
#   are signed like every other Mach-O. Every Mach-O's load commands are then checked
#   (lib/bundle-check.sh): nothing loaded from outside the bundle and the OS (Homebrew is on the
#   build machine, not the user's), no absolute rpath, every library resolvable inside the bundle,
#   nothing built for a newer macOS than the app's. Each bundled library must be one THIRD_PARTY.md
#   covers: a new one stops the build until its licence has been read and listed.
# - A verification that cannot fail the build is not one: every check below exits non-zero.
set -euo pipefail

mac="$(cd "$(dirname "$0")/.." && pwd)"
# redact_signing: everything signing-related this script prints goes through it.
. "$mac/scripts/lib/redact-signing.sh"
# check_bundle_linkage: what the bundle's code loads, and from where.
. "$mac/scripts/lib/bundle-check.sh"
config=release
skip_core=0
same_as=""
timestamp=0
engines=0
while [ $# -gt 0 ]; do
  case "$1" in
    --debug) config=debug ;;
    --release) config=release ;;
    --skip-core) skip_core=1 ;;
    --timestamp) timestamp=1 ;;
    --engines) engines=1 ;;
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
target="${MACOSX_DEPLOYMENT_TARGET:-26.0}"
allow_newer="${INK_ALLOW_NEWER_MACOS:-0}"
# The engines a release ships: the dictation and meeting finals (Qwen3-ASR on llama.cpp), the VAD
# (Silero on tract) and the far end's diarizer (Nemotron on NeMo-Speech.cpp).
release_features="engine-llama,ink-engines/engine-silero,ink-engines/engine-nemo"
link_file="$mac/build/InkCore.link"

# A release without its engines would install and start, and transcribe nothing.
if [ "$timestamp" = 1 ]; then
  [ "$engines" = 1 ] || fail "--timestamp builds a release, and a release ships its engines: add --engines"
  [ "$allow_newer" != 1 ] || fail "INK_ALLOW_NEWER_MACOS is for local builds: a release must start on macOS $target"
fi
if [ "$engines" = 1 ]; then
  if [ -n "${INK_CORE_FEATURES:-}" ] && [ "$INK_CORE_FEATURES" != "$release_features" ]; then
    fail "--engines builds the core with $release_features; INK_CORE_FEATURES asks for $INK_CORE_FEATURES"
  fi
  export INK_CORE_FEATURES="$release_features"
fi

if [ -n "$identity" ]; then
  # Checked before building, so a typo fails in seconds. Listing identities reads certificates,
  # not private keys: it never shows a keychain prompt. (The codesign calls below may.) Read whole
  # before matching: grep -q closing the pipe early could fail `security` under pipefail, and
  # with it a valid identity.
  identities="$(security find-identity -v -p codesigning)"
  grep -qF -- "$identity" <<<"$identities" \
    || fail "INK_SIGN_IDENTITY names no valid code-signing identity in the keychain"
else
  [ "$timestamp" = 0 ] || fail "--timestamp needs INK_SIGN_IDENTITY: an ad-hoc signature has no timestamp"
  echo "build-mac: INK_SIGN_IDENTITY is not set: signing ad-hoc. Fine for a build check; do not" >&2
  echo "           grant this build any permission (TCC forgets it at the next build)." >&2
fi

# --- build --------------------------------------------------------------------------------------
if [ "$skip_core" = 1 ]; then
  [ -d "$mac/build/InkCore.xcframework" ] || fail "--skip-core, but mac/build/InkCore.xcframework is missing"
else
  "$mac/scripts/build-core.sh"
fi
# The linker arguments this build of the core needs (build-core.sh wrote them beside it), and the
# directories its engines' libraries are bundled from (the -L ones).
[ -f "$link_file" ] || fail "mac/build/InkCore.link is missing: build the core again (without --skip-core)"
core_features="$(sed -n 's/^# features: //p' "$link_file")"
if [ "$engines" = 1 ] && [ "$core_features" != "$release_features" ]; then
  fail "--engines, but the core in mac/build was built with features [$core_features]: build it again"
fi
link_args=()
dylib_dirs=()
while IFS= read -r arg; do
  case "$arg" in
    '' | '#'*) continue ;;
    -L*) dylib_dirs+=("${arg#-L}") ;;
  esac
  link_args+=(-Xlinker "$arg")
done <"$link_file"
# Only the versions pinned in the committed Package.resolved: a build that re-resolved could ship
# a dependency the licence audit never saw. A stale or missing Package.resolved fails here.
# The rpath: the frameworks and libraries the app loads (Sparkle, the engines') go in
# Contents/Frameworks, and SwiftPM's own rpath is only @loader_path, which is Contents/MacOS in the
# bundle.
swift build --package-path "$mac" -c "$config" --product Inkwell --only-use-versions-from-resolved-file \
  -Xlinker -rpath -Xlinker @executable_path/../Frameworks ${link_args[@]+"${link_args[@]}"}
bin="$(swift build --package-path "$mac" -c "$config" --show-bin-path --only-use-versions-from-resolved-file)"

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
# A library the executable (or a bundled library) loads as @rpath/<name>: the engines'. Copied into
# Contents/Frameworks under that name from the directories the core was linked against (a link
# there is followed: the file goes in, under the name it is loaded by), then whatever it loads
# through @rpath in turn. Only the libraries THIRD_PARTY.md covers, by name.
bundle_dylib() {
  local name="$1" dir src="" ref
  [ -e "$app/Contents/Frameworks/$name" ] && return 0
  case "$name" in
    # NeMo-Speech.cpp, its ggml, SentencePiece (with protobuf-lite and Darts-clone) and Abseil.
    libnemo_speech_asr*.dylib | libggml*.dylib | libsentencepiece*.dylib | libabsl_*.dylib) ;;
    *) fail "the app loads @rpath/$name, which THIRD_PARTY.md does not cover: read its licence and list it first" ;;
  esac
  for dir in ${dylib_dirs[@]+"${dylib_dirs[@]}"}; do
    if [ -e "$dir/$name" ]; then
      src="$dir/$name"
      break
    fi
  done
  [ -n "$src" ] || fail "the app loads @rpath/$name, which no library directory of the core holds"
  mkdir -p "$app/Contents/Frameworks"
  cp "$src" "$app/Contents/Frameworks/$name"
  chmod u+w "$app/Contents/Frameworks/$name"
  while IFS= read -r ref; do
    case "$ref" in
      @rpath/*.dylib) bundle_dylib "${ref#@rpath/}" ;;
    esac
  done < <(otool -L "$app/Contents/Frameworks/$name" | sed -nE '2,$ s/^[[:space:]]+([^ ]+) \(.*/\1/p')
}

# The frameworks the executable links through its rpath, as SwiftPM unpacked them from their
# XCFrameworks next to it. Read from the executable, so a framework left in the build directory by
# an earlier dependency is not shipped. ditto keeps the symlinks a framework's signature covers.
while IFS= read -r ref; do
  case "$ref" in
    @rpath/*.framework/*)
      framework="${ref#@rpath/}"
      framework="${framework%%.framework/*}.framework"
      [ -d "$bin/$framework" ] || fail "the app links $framework, which the build did not produce"
      [ -d "$app/Contents/Frameworks/$framework" ] || ditto "$bin/$framework" "$app/Contents/Frameworks/$framework"
      ;;
    @rpath/*.dylib) bundle_dylib "${ref#@rpath/}" ;;
    @rpath/*) fail "the app links $ref through its rpath, which is neither a framework nor a dylib" ;;
  esac
done < <(otool -L "$bin/Inkwell" | sed -nE '2,$ s/^[[:space:]]+([^ ]+) \(.*/\1/p')
bundled=0
if [ -d "$app/Contents/Frameworks" ]; then
  bundled="$(find "$app/Contents/Frameworks" -maxdepth 1 -name '*.dylib' -type f | wc -l | tr -d ' ')"
fi

# What every Mach-O loads, and from where: before anything is signed.
check_bundle_linkage "$app" "$target" "$allow_newer" || fail "the bundle's code loads what a user's Mac may not have (above)"

# --- sign ---------------------------------------------------------------------------------------
sign_as=("${identity:--}")
# --timestamp=none: a local build does not ask Apple's timestamp server. The release pipeline
# passes --timestamp: notarisation requires a secure timestamp on every signature.
stamp=(--timestamp=none)
[ "$timestamp" = 1 ] && stamp=(--timestamp)
# codesign's own messages can name the identity and its certificate: they are redacted too
# (pipefail keeps its exit status).
sign() {
  codesign --force --options runtime "${stamp[@]}" --sign "${sign_as[0]}" "$@" 2>&1 \
    | redact_signing "$identity"
}

is_macho() { file -b "$1" | grep -q '^Mach-O'; }

# A framework: its own code first, deepest first, then the framework. Sparkle's own code is two
# XPC services, the Updater app and the Autoupdate tool, all signed without entitlements: the app
# is not sandboxed, so Sparkle installs through Autoupdate and Updater.app as ordinary helpers and
# never starts its XPC services (they serve sandboxed apps, which opt in through Info.plist).
# Whatever in a framework this misses is still caught below, still signed by its vendor.
sign_framework() {
  local framework="$1" version code f
  [ -L "$framework/Versions/Current" ] || fail "${framework#"$app/"} has no Versions/Current"
  version="$framework/Versions/$(readlink "$framework/Versions/Current")"
  for code in "$version/XPCServices/"*.xpc "$version/"*.app; do
    [ -d "$code" ] && sign "$code"
  done
  for f in "$version/"*; do
    [ -f "$f" ] && [ ! -L "$f" ] || continue
    [ "$(basename "$f")" = "$(basename "$framework" .framework)" ] && continue
    is_macho "$f" && sign "$f"
  done
  sign "$framework"
}

# Inside out: resource bundles, then every loose Mach-O other than the main executable, then the
# frameworks, then the app itself with its entitlements. A Mach-O inside any other nested bundle
# has to be signed as part of that bundle; nothing produces one yet, so it stops the build rather
# than going out half signed.
nested=()
while IFS= read -r -d '' f; do
  # Matched on the path inside the bundle: the bundle's own path ends in .app too.
  case "${f#"$app/Contents/"}" in
    MacOS/Inkwell) continue ;;
    Frameworks/*.framework/*) continue ;;
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
frameworks=0
for framework in "$app/Contents/Frameworks/"*.framework; do
  [ -d "$framework" ] || continue
  sign_framework "$framework"
  frameworks=$((frameworks + 1))
done
# Ad-hoc code has no team, and the hardened runtime's library validation loads a framework only
# from the app's own team, so an ad-hoc app could not load its own ad-hoc Sparkle ("different Team
# IDs"). An ad-hoc build is a local check that TCC forgets at the next build anyway: it alone turns
# library validation off. A Developer ID build never does, and is checked for that below.
entitlements="$mac/Inkwell.entitlements"
if [ -z "$identity" ]; then
  entitlements="$mac/build/adhoc-entitlements.plist"
  cp "$mac/Inkwell.entitlements" "$entitlements"
  /usr/libexec/PlistBuddy -c 'Add :com.apple.security.cs.disable-library-validation bool true' "$entitlements" >/dev/null
fi
sign --entitlements "$entitlements" "$app"

# --- verify -------------------------------------------------------------------------------------
codesign --verify --deep --strict "$app" 2>&1 | redact_signing "$identity" || fail "the signature does not verify"

info="$(codesign -dv "$app" 2>&1)"
grep -q "^Identifier=$bundle_id\$" <<<"$info" || fail "the app is not signed as $bundle_id"
grep -Eq '^CodeDirectory .*flags=0x[0-9a-f]+\([^)]*runtime' <<<"$info" || fail "the hardened runtime is off"

signed_ents="$mac/build/signed-entitlements.plist"
codesign -d --entitlements - --xml "$app" >"$signed_ents" 2>/dev/null || fail "no entitlements could be read back"
# PlistBuddy, not plutil: plutil's key paths split on the dots in the key.
[ "$(/usr/libexec/PlistBuddy -c 'Print :com.apple.security.device.audio-input' "$signed_ents" 2>/dev/null)" = true ] \
  || fail "the audio-input entitlement is missing: the hardened runtime would deny the microphone silently"
if [ -n "$identity" ] \
  && /usr/libexec/PlistBuddy -c 'Print :com.apple.security.cs.disable-library-validation' "$signed_ents" >/dev/null 2>&1; then
  fail "a Developer ID build carries disable-library-validation: only an ad-hoc build may"
fi

# Every Mach-O in the bundle, the frameworks' helpers included (for one inside a nested bundle,
# codesign reads that bundle's signature): same kind of signature as the app (ad-hoc or not), same
# team, the hardened runtime, a secure timestamp when asked for, and no entitlements but the app's.
team_of() { codesign -dv "$1" 2>&1 | sed -n 's/^TeamIdentifier=//p'; }
app_team="$(team_of "$app")"
machos=0
while IFS= read -r -d '' f; do
  is_macho "$f" || continue
  machos=$((machos + 1))
  details="$(codesign -dv "$f" 2>&1)" || fail "${f#"$app/"} is not signed"
  if [ -n "$identity" ]; then
    grep -q 'Signature=adhoc' <<<"$details" && fail "${f#"$app/"} is signed ad-hoc"
    [ "$(team_of "$f")" = "$app_team" ] || fail "${f#"$app/"} is signed by another team than the app"
  fi
  grep -Eq 'flags=0x[0-9a-f]+\([^)]*runtime' <<<"$details" || fail "${f#"$app/"} lacks the hardened runtime"
  if [ "$timestamp" = 1 ]; then
    grep -q '^Timestamp=' <<<"$details" || fail "${f#"$app/"} has no secure timestamp"
  fi
  if [ "$f" != "$app/Contents/MacOS/Inkwell" ]; then
    # Read whole before matching: grep -q closing a pipe early would fail codesign under pipefail.
    helper_ents="$(codesign -d --entitlements - --xml "$f" 2>/dev/null)" || fail "${f#"$app/"}: no entitlements read"
    grep -q '<key>' <<<"$helper_ents" && fail "${f#"$app/"} carries entitlements: only the app may"
  fi
done < <(find "$app/Contents" -type f -print0)

if [ -n "$same_as" ]; then
  # Compared in full, printed only redacted: a Developer ID requirement names the team, and can
  # name the certificate. An ad-hoc signature's requirement is implicit, printed as
  # "# designated => cdhash ...".
  requirement() { codesign -d -r- "$1" 2>&1 | sed -nE 's/^(# )?designated => //p'; }
  ours="$(requirement "$app")"
  theirs="$(requirement "$same_as")"
  [ -n "$theirs" ] || fail "no designated requirement read from $same_as"
  if [ "$ours" != "$theirs" ]; then
    echo "  this build: $(redact_signing "$identity" <<<"$ours")" >&2
    echo "  $same_as: $(redact_signing "$identity" <<<"$theirs")" >&2
    fail "the designated requirement differs from $same_as: TCC grants will not carry over"
  fi
  echo "designated requirement: the same as $same_as"
fi

summary="$machos Mach-O file(s): ${#nested[@]} loose (of them $bundled the engines' libraries), $frameworks framework(s)"
if [ -n "$identity" ]; then
  echo "signed: INK_SIGN_IDENTITY$([ "$timestamp" = 1 ] && echo ', secure timestamp'), hardened runtime, audio-input; $summary"
else
  echo "signed: ad-hoc (build check only), hardened runtime, audio-input, library validation off; $summary"
fi
echo "built: $app ($config)"
