#!/usr/bin/env bash
# check_pinned_origins (core/crates/ink-engines/native/lib/self-contained-prefix.sh), which
# build-nemo-speech.sh runs after make_self_contained when ENGINE_DEPS_DIR names the pinned
# SentencePiece and Abseil: the prefix must have copied in both, every copy from that prefix.
# Tiny libraries compiled here with clang stand in for NeMo's, SentencePiece's and Abseil's.
# Offline.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/assert.sh"
library="$(cd "$here/../../../core/crates/ink-engines/native/lib" && pwd)/self-contained-prefix.sh"

# The real path: the walk records each copy's source as realpath gives it (/var is a link).
work="$(cd "$(mktemp -d "${TMPDIR:-/tmp}/ink-pinned-origins-test.XXXXXX")" && pwd -P)"
trap 'rm -rf "$work"' EXIT

cc() { xcrun clang -mmacosx-version-min=26.0 "$@"; }

# The pinned prefix, as build-sentencepiece-abseil.sh leaves it: @rpath install names, and
# SentencePiece loading Abseil beside it. And another Abseil, elsewhere, by an absolute name (as
# Homebrew's are).
deps="$work/deps/lib"
other="$work/other/lib"
mkdir -p "$work/src" "$deps" "$other"
echo 'int absl(void) { return 1; }' >"$work/src/absl.c"
echo 'int absl(void); int spm(void) { return 1 + absl(); }' >"$work/src/spm.c"
cc -dynamiclib -install_name @rpath/libabsl_status.1.dylib -o "$deps/libabsl_status.1.dylib" "$work/src/absl.c"
cc -dynamiclib -install_name @rpath/libsentencepiece.0.dylib -o "$deps/libsentencepiece.0.dylib" \
  "$work/src/spm.c" "$deps/libabsl_status.1.dylib" -Wl,-rpath,@loader_path
cc -dynamiclib -install_name "$other/libabsl_log.1.dylib" -o "$other/libabsl_log.1.dylib" "$work/src/absl.c"

# prefix <name> <link arguments...>: a NeMo prefix whose one library, libn, links those, with the
# pinned prefix as an rpath (as CMAKE_INSTALL_RPATH_USE_LINK_PATH gives NeMo's).
prefix() {
  local d="$work/$1"
  shift
  mkdir -p "$d/lib"
  echo 'int n(void) { return 0; }' >"$work/src/n.c"
  cc -dynamiclib -install_name @rpath/libn.dylib -o "$d/lib/libn.dylib" "$work/src/n.c" "$@" \
    -Wl,-rpath,"$deps" -Wl,-rpath,@loader_path
}

# check <name>: make_self_contained, then check_pinned_origins, as build-nemo-speech.sh runs them
# (under set -e, in its own bash). Sets $status and $out.
check() {
  local d="$work/$1"
  status=0
  out="$(/bin/bash -c 'set -euo pipefail; . "$1"
    make_self_contained "$2/lib" 26.0 "$2/lib/libn.dylib"
    check_pinned_origins "$3"' _ "$library" "$d" "$deps" 2>&1)" || status=$?
}

prefix all "$deps/libsentencepiece.0.dylib" "$deps/libabsl_status.1.dylib"
check all
assert_status "SentencePiece and Abseil, both copied from the pinned prefix" 0 "$status"
[ -f "$work/all/lib/libsentencepiece.0.dylib" ] && [ -f "$work/all/lib/libabsl_status.1.dylib" ] \
  && pass "... both are in the NeMo prefix" || flunk "... a copy is missing: $(ls "$work/all/lib")"

prefix elsewhere "$deps/libsentencepiece.0.dylib" "$deps/libabsl_status.1.dylib" "$other/libabsl_log.1.dylib"
check elsewhere
assert_status "one library copied from elsewhere" 1 "$status"
assert_contains "... says which, and from where" "$out" \
  "libabsl_log.1.dylib was copied from $other/libabsl_log.1.dylib, not from ENGINE_DEPS_DIR"

prefix none
check none
assert_status "nothing copied in (NeMo no longer loads them)" 1 "$status"
assert_contains "... says so" "$out" "copied in no SentencePiece or no Abseil library"

prefix absl-only "$deps/libabsl_status.1.dylib"
check absl-only
assert_status "Abseil copied in, but no SentencePiece" 1 "$status"
assert_contains "... says so" "$out" "copied in no SentencePiece or no Abseil library"

finish
