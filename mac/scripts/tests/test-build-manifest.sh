#!/usr/bin/env bash
# build-manifest.sh: the record a Mac release keeps beside its dmg, of the libraries it bundles, the
# pinned sources they were built from and the build tools' versions. It is published, so it must
# name no path on the build machine; and it must agree with itself, or refuse to be written.
# Fixtures only, offline.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/assert.sh"
script="$here/../build-manifest.sh"
deps_script="$here/../../../core/crates/ink-engines/native/build-sentencepiece-abseil.sh"

work="$(mktemp -d "${TMPDIR:-/tmp}/ink-build-manifest-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT
work="$(cd "$work" && pwd -P)"

commit=0123456789abcdef0123456789abcdef01234567
printf 'not really a dmg\n' >"$work/Inkwell_1.2.3_aarch64.dmg"
dmg_sha="$(shasum -a 256 <"$work/Inkwell_1.2.3_aarch64.dmg" | cut -d' ' -f1)"
printf '%s\n' 'cmake 4.1.2' 'ninja 1.13.1 1.13.2' >"$work/brew.txt"

# The pins, read from the build script (never retyped here).
pin() { sed -n "s/^$1=//p" "$deps_script"; }
absl_version="$(pin ABSEIL_VERSION)"
spm_version="$(pin SENTENCEPIECE_VERSION)"
absl_source="abseil $absl_version $(pin ABSEIL_SHA256) $(pin ABSEIL_TARBALL)"
spm_source="sentencepiece $spm_version $(pin SENTENCEPIECE_SHA256) $(pin SENTENCEPIECE_TARBALL)"

# A pinned prefix as build-sentencepiece-abseil.sh leaves it: two libraries and its manifest.
deps="$work/deps"
mkdir -p "$deps/lib" "$deps/share/inkwell"
printf 'sentencepiece\n' >"$deps/lib/libsentencepiece.0.0.0.dylib"
printf 'absl base\n' >"$deps/lib/libabsl_base.2608.0.0.dylib"
printf 'absl extra\n' >"$deps/lib/libabsl_extra.2608.0.0.dylib"
sha() { shasum -a 256 <"$1" | cut -d' ' -f1; }
# deps_manifest <file> <source lines...>: the prefix's manifest with those sources.
deps_manifest() {
  local file=$1
  shift
  {
    echo "# SentencePiece and Abseil as built by build-sentencepiece-abseil.sh, from pinned tarballs."
    printf 'source %s\n' "$@"
    echo "deployment_target 26.0"
    echo "lib abseil $(sha "$deps/lib/libabsl_base.2608.0.0.dylib") lib/libabsl_base.2608.0.0.dylib"
    echo "lib sentencepiece $(sha "$deps/lib/libsentencepiece.0.0.0.dylib") lib/libsentencepiece.0.0.0.dylib"
  } >"$file"
}
deps_manifest "$deps/share/inkwell/engine-deps.manifest" "$absl_source" "$spm_source"

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
  "# bundled libsentencepiece.0.dylib $deps/lib/libsentencepiece.0.0.0.dylib"
  "# bundled libabsl_base.2608.0.0.dylib $deps/lib/libabsl_base.2608.0.0.dylib"
)

# run <label> <expected status> <nemo manifest> [extra arguments...]: sets $out (what it printed)
# and leaves the manifest, if any, at $work/out.txt.
run() {
  local label=$1 want=$2 manifest=$3 status=0
  shift 3
  rm -f "$work/out.txt"
  out="$(/bin/bash "$script" --version 1.2.3 --commit "$commit" --dmg "$work/Inkwell_1.2.3_aarch64.dmg" \
    --nemo "$manifest" --deps "$deps/share/inkwell/engine-deps.manifest" --homebrew "$work/brew.txt" \
    --out "$work/out.txt" "$@" 2>&1)" || status=$?
  assert_status "$label" "$want" "$status"
  if [ "$want" != 0 ]; then
    [ ! -e "$work/out.txt" ] && pass "$label: writes nothing" || flunk "$label: wrote $work/out.txt"
  fi
}

nemo "$work/good.manifest" "${good_origins[@]}"
run "a prefix built from the pinned SentencePiece and Abseil" 0 "$work/good.manifest"
m="$(cat "$work/out.txt")"
assert_contains "the version" "$m" "version 1.2.3"
assert_contains "the source commit" "$m" "source $commit"
assert_contains "the dmg and its SHA-256" "$m" "dmg Inkwell_1.2.3_aarch64.dmg sha256 $dmg_sha"
assert_contains "NeMo's commit" "$m" "nemo_speech commit 97a15afa5caa9bce5baaa86c1184103877af4101"
assert_contains "NeMo's GGML_NATIVE" "$m" "nemo_speech ggml_native OFF"
assert_contains "each prefix library's SHA-256" "$m" \
  "nemo_speech sha256 1111111111111111111111111111111111111111111111111111111111111111 lib/libabsl_base.2608.0.0.dylib"
assert_contains "Abseil's pinned version and tarball" "$m" "pinned $absl_source"
assert_contains "SentencePiece's pinned version and tarball" "$m" "pinned $spm_source"
assert_contains "a bundled library: project, pinned version, file" "$m" \
  "bundled libsentencepiece.0.dylib sentencepiece $spm_version lib/libsentencepiece.0.0.0.dylib"
assert_contains "... each one" "$m" \
  "bundled libabsl_base.2608.0.0.dylib abseil $absl_version lib/libabsl_base.2608.0.0.dylib"
assert_contains "the build tools' versions" "$m" "homebrew ninja 1.13.1 1.13.2"
assert_absent "no path on the build machine: the pinned prefix's" "$m" "$deps"
assert_absent "... nor the dmg's directory" "$m" "$work"

nemo "$work/brew.manifest" "${good_origins[@]}" \
  "# bundled libabsl_log.2608.0.0.dylib /opt/homebrew/Cellar/abseil/20260817.0/lib/libabsl_log.2608.0.0.dylib"
run "a library from Homebrew" 1 "$work/brew.manifest"
assert_contains "... says why" "$out" "libabsl_log.2608.0.0.dylib was copied from outside the pinned SentencePiece/Abseil prefix"
assert_absent "... naming no path" "$out" "/opt/homebrew"

nemo "$work/unlisted.manifest" "${good_origins[@]}" "# bundled libabsl_extra.2608.0.0.dylib $deps/lib/libabsl_extra.2608.0.0.dylib"
run "a library in the pinned prefix its manifest does not list" 1 "$work/unlisted.manifest"
assert_contains "... says why" "$out" "which the pinned prefix's manifest does not list"

cp "$deps/lib/libabsl_base.2608.0.0.dylib" "$work/base.keep"
printf 'swapped\n' >"$deps/lib/libabsl_base.2608.0.0.dylib"
run "a pinned library changed after the build" 1 "$work/good.manifest"
assert_contains "... says why" "$out" "changed after the pinned prefix was built"
cp "$work/base.keep" "$deps/lib/libabsl_base.2608.0.0.dylib"

cp "$deps/share/inkwell/engine-deps.manifest" "$work/deps.keep"
deps_manifest "$deps/share/inkwell/engine-deps.manifest" "$absl_source" \
  "sentencepiece $spm_version 0000000000000000000000000000000000000000000000000000000000000000 $(pin SENTENCEPIECE_TARBALL)"
run "a prefix built from another SentencePiece tarball" 1 "$work/good.manifest"
assert_contains "... says why" "$out" "was not built from the pinned sentencepiece"
deps_manifest "$deps/share/inkwell/engine-deps.manifest" "$absl_source"
run "a prefix that records one source" 1 "$work/good.manifest"
assert_contains "... says why" "$out" "records 1 source(s), not the 2 pinned ones"
{ cat "$work/deps.keep"; echo "lib abseil nothex lib/x.dylib"; } >"$deps/share/inkwell/engine-deps.manifest"
run "an unreadable line in the pinned prefix's manifest" 1 "$work/good.manifest"
assert_contains "... says why" "$out" "unreadable line"
cp "$work/deps.keep" "$deps/share/inkwell/engine-deps.manifest"

run "no pinned prefix's manifest" 1 "$work/good.manifest" --deps "$work/missing.manifest"
assert_contains "... says why" "$out" "no SentencePiece/Abseil manifest"

# A copy of the manifest outside its prefix: the prefix it names cannot be found from it.
mkdir -p "$work/flat"
cp "$deps/share/inkwell/engine-deps.manifest" "$work/flat/engine-deps.manifest"
run "a pinned prefix's manifest outside <prefix>/share/inkwell" 1 "$work/good.manifest" \
  --deps "$work/flat/engine-deps.manifest"
assert_contains "... says why" "$out" "is not in <prefix>/share/inkwell/"

nemo "$work/odd.manifest" "${good_origins[@]}" "ggml_native"
run "an unreadable manifest line" 1 "$work/odd.manifest"
assert_contains "... says why" "$out" "unreadable line"

nemo "$work/abs.manifest" "${good_origins[@]}" "sha256 3333333333333333333333333333333333333333333333333333333333333333 /opt/x/lib/libx.dylib"
run "a library hash of a file outside the prefix" 1 "$work/abs.manifest"
assert_contains "... says why" "$out" "unreadable line"

# Every input is checked on its own, but the last guard stands behind them: a value no earlier
# check reads (NeMo's commit here) that names a path stops the file from being written.
sed 's|^commit .*|commit /opt/build/nemo|' "$work/good.manifest" >"$work/pathcommit.manifest"
run "an input that would put an absolute path in the public file" 1 "$work/pathcommit.manifest"
assert_contains "... the last guard says why" "$out" "the manifest would name an absolute path"

grep -v '^commit ' "$work/good.manifest" >"$work/nocommit.manifest"
run "a manifest without NeMo's commit" 1 "$work/nocommit.manifest"
assert_contains "... says why" "$out" "no commit"

printf '%s\n' 'cmake' >"$work/brew-bad.txt"
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
