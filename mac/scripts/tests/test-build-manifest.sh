#!/usr/bin/env bash
# build-manifest.sh: the record a Mac release keeps beside its dmg, of the libraries it bundles and
# the Homebrew versions it was built with. It is published, so it must name no path on the build
# machine; and it must agree with itself, or refuse to be written. Fixtures only, offline.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/assert.sh"
script="$here/../build-manifest.sh"

work="$(mktemp -d "${TMPDIR:-/tmp}/ink-build-manifest-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT

commit=0123456789abcdef0123456789abcdef01234567
printf 'not really a dmg\n' >"$work/Inkwell_1.2.3_aarch64.dmg"
dmg_sha="$(shasum -a 256 <"$work/Inkwell_1.2.3_aarch64.dmg" | cut -d' ' -f1)"
printf '%s\n' 'abseil 20260817.0' 'cmake 4.1.2' 'ninja 1.13.1' 'sentencepiece 0.2.1 0.2.2_1' >"$work/brew.txt"

# nemo <file> <extra lines...>: a prefix manifest as build-nemo-speech.sh writes it.
nemo() {
  local file=$1
  shift
  {
    echo "# NeMo-Speech.cpp as installed by build-nemo-speech.sh; ink-engines' build.rs checks it."
    echo "commit 97a15afa5caa9bce5baaa86c1184103877af4101"
    echo "ggml_native OFF"
    echo "# Copied into lib/ from outside the OS (each library's source, for its licence):"
    printf '%s\n' "$@"
    echo "sha256 1111111111111111111111111111111111111111111111111111111111111111 lib/libabsl_base.2608.0.0.dylib"
    echo "sha256 2222222222222222222222222222222222222222222222222222222222222222 lib/libnemo_speech_asr_c.dylib"
  } >"$file"
}
good_origins=(
  "# bundled libsentencepiece.0.dylib /opt/homebrew/Cellar/sentencepiece/0.2.2_1/lib/libsentencepiece.0.0.0.dylib"
  "# bundled libabsl_base.2608.0.0.dylib /usr/local/Cellar/abseil/20260817.0/lib/libabsl_base.2608.0.0.dylib"
)

# run <label> <expected status> <nemo manifest> [extra arguments...]: sets $out (what it printed)
# and leaves the manifest, if any, at $work/out.txt.
run() {
  local label=$1 want=$2 manifest=$3 status=0
  shift 3
  rm -f "$work/out.txt"
  out="$(/bin/bash "$script" --version 1.2.3 --commit "$commit" --dmg "$work/Inkwell_1.2.3_aarch64.dmg" \
    --nemo "$manifest" --homebrew "$work/brew.txt" --out "$work/out.txt" "$@" 2>&1)" || status=$?
  assert_status "$label" "$want" "$status"
  if [ "$want" != 0 ]; then
    [ ! -e "$work/out.txt" ] && pass "$label: writes nothing" || flunk "$label: wrote $work/out.txt"
  fi
}

nemo "$work/good.manifest" "${good_origins[@]}"
run "a prefix built from Homebrew" 0 "$work/good.manifest"
m="$(cat "$work/out.txt")"
assert_contains "the version" "$m" "version 1.2.3"
assert_contains "the source commit" "$m" "source $commit"
assert_contains "the dmg and its SHA-256" "$m" "dmg Inkwell_1.2.3_aarch64.dmg sha256 $dmg_sha"
assert_contains "NeMo's commit" "$m" "nemo_speech commit 97a15afa5caa9bce5baaa86c1184103877af4101"
assert_contains "NeMo's GGML_NATIVE" "$m" "nemo_speech ggml_native OFF"
assert_contains "each prefix library's SHA-256" "$m" \
  "nemo_speech sha256 1111111111111111111111111111111111111111111111111111111111111111 lib/libabsl_base.2608.0.0.dylib"
assert_contains "a bundled library: formula, version, file in its keg" "$m" \
  "bundled libsentencepiece.0.dylib sentencepiece 0.2.2_1 lib/libsentencepiece.0.0.0.dylib"
assert_contains "... from either Homebrew prefix" "$m" \
  "bundled libabsl_base.2608.0.0.dylib abseil 20260817.0 lib/libabsl_base.2608.0.0.dylib"
assert_contains "Homebrew's versions" "$m" "homebrew sentencepiece 0.2.1 0.2.2_1"
assert_contains "... every package" "$m" "homebrew cmake 4.1.2"
assert_absent "no path on the build machine: Homebrew's" "$m" "/opt/homebrew"
assert_absent "... nor its Cellar" "$m" "Cellar"
assert_absent "... nor the dmg's directory" "$m" "$work"

nemo "$work/elsewhere.manifest" "${good_origins[@]}" "# bundled libfoo.dylib /opt/local/lib/libfoo.dylib"
run "a library from outside a Homebrew keg" 1 "$work/elsewhere.manifest"
assert_contains "... says why" "$out" "libfoo.dylib was copied from outside a Homebrew keg"

nemo "$work/unlisted.manifest" "# bundled libabsl_base.2608.0.0.dylib /opt/homebrew/Cellar/abseil/20250814.1/lib/libabsl_base.2608.0.0.dylib"
run "a keg version Homebrew does not list" 1 "$work/unlisted.manifest"
assert_contains "... says why" "$out" "abseil 20250814.1, which brew list --versions does not report"

nemo "$work/odd.manifest" "${good_origins[@]}" "ggml_native"
run "an unreadable manifest line" 1 "$work/odd.manifest"
assert_contains "... says why" "$out" "unreadable line"

nemo "$work/abs.manifest" "${good_origins[@]}" "sha256 3333333333333333333333333333333333333333333333333333333333333333 /opt/x/lib/libx.dylib"
run "a library hash of a file outside the prefix" 1 "$work/abs.manifest"
assert_contains "... says why" "$out" "unreadable line"

grep -v '^commit ' "$work/good.manifest" >"$work/nocommit.manifest"
run "a manifest without NeMo's commit" 1 "$work/nocommit.manifest"
assert_contains "... says why" "$out" "no commit"

printf '%s\n' 'abseil' >"$work/brew-bad.txt"
run "an unreadable Homebrew list" 1 "$work/good.manifest" --homebrew "$work/brew-bad.txt"
assert_contains "... says why" "$out" "not a line of brew list --versions"

run "a dmg that is not there" 1 "$work/good.manifest" --dmg "$work/missing.dmg"
assert_contains "... says why" "$out" "no dmg at"

run "a version that is not X.Y.Z" 1 "$work/good.manifest" --version 1.2
assert_contains "... says why" "$out" "--version is X.Y.Z"

run "a commit that is not a full SHA-1" 1 "$work/good.manifest" --commit 0123abc
assert_contains "... says why" "$out" "--commit is a full commit hash"

run "an unknown argument" 1 "$work/good.manifest" --publish
assert_contains "... says why" "$out" "unknown argument"

finish
