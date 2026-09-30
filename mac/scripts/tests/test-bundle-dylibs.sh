#!/usr/bin/env bash
# lib/bundle-dylibs.py, which build-mac.sh's bundle_dylib runs: each library the app loads through
# @rpath, and each one those load in turn, is copied into Contents/Frameworks under the name it is
# loaded by (a link in the library directory is followed), from the first directory that holds it,
# and only if its name is allowed. Tiny fixture libraries are compiled here with clang.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/assert.sh"
bundler="$here/../lib/bundle-dylibs.py"

work="$(mktemp -d "${TMPDIR:-/tmp}/ink-bundle-dylibs-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/empty" "$work/lib"

# lib <file> <name> [libraries it links...]: a library in $work/lib, installed as @rpath/<name>.
lib() {
  local file=$1 name=$2
  shift 2
  echo "int ${file//[^a-z]/_}(void) { return 1; }" >"$work/$file.c"
  xcrun clang -dynamiclib -mmacosx-version-min=26.0 -install_name "@rpath/$name" \
    -o "$work/lib/$file" "$work/$file.c" "$@"
}
# libabsl_b.dylib is a link to the library's file, as a library directory versions its libraries.
lib libabsl_b.1.dylib libabsl_b.dylib
ln -s libabsl_b.1.dylib "$work/lib/libabsl_b.dylib"
lib libabsl_a.dylib libabsl_a.dylib "$work/lib/libabsl_b.dylib"
chmod a-w "$work/lib/libabsl_a.dylib"
lib libother.dylib libother.dylib
lib libabsl_c.dylib libabsl_c.dylib "$work/lib/libother.dylib"

# run <Frameworks directory> <name...>: the bundler with libabsl_* allowed, from both directories.
run() {
  local frameworks=$1
  shift
  python3 "$bundler" --into "$frameworks" --allow 'libabsl_*.dylib' --from "$work/empty" --from "$work/lib" -- "$@"
}

out="$(run "$work/ok/Frameworks" libabsl_a.dylib 2>&1)" || flunk "an allowed library and what it loads: exit status $?: $out"
assert_contains "the library the app loads is bundled" "$(ls "$work/ok/Frameworks")" "libabsl_a.dylib"
assert_contains "... and the one it loads, under the name it loads it by" "$(ls "$work/ok/Frameworks")" "libabsl_b.dylib"
assert_absent "... and not under the link's target name" "$(ls "$work/ok/Frameworks")" "libabsl_b.1.dylib"
if [ -f "$work/ok/Frameworks/libabsl_b.dylib" ] && [ ! -L "$work/ok/Frameworks/libabsl_b.dylib" ]; then
  pass "a link in the library directory goes in as the file behind it"
else
  flunk "a link in the library directory goes in as the file behind it"
fi
if [ -w "$work/ok/Frameworks/libabsl_a.dylib" ]; then
  pass "a read-only library is writable in the bundle (codesign rewrites it)"
else
  flunk "a read-only library is writable in the bundle (codesign rewrites it)"
fi

status=0
out="$(run "$work/other/Frameworks" libother.dylib 2>&1)" || status=$?
assert_status "a library THIRD_PARTY.md does not cover: fails" 1 "$status"
assert_contains "... and says why" "$out" \
  "build-mac: the app loads @rpath/libother.dylib, which THIRD_PARTY.md does not cover: read its licence and list it first"

status=0
out="$(run "$work/deep/Frameworks" libabsl_c.dylib 2>&1)" || status=$?
assert_status "an allowed library that loads one not covered: fails" 1 "$status"
assert_contains "... and names that one" "$out" "the app loads @rpath/libother.dylib, which THIRD_PARTY.md does not cover"

status=0
out="$(run "$work/gone/Frameworks" libabsl_gone.dylib 2>&1)" || status=$?
assert_status "a library no directory holds: fails" 1 "$status"
assert_contains "... and says why" "$out" \
  "build-mac: the app loads @rpath/libabsl_gone.dylib, which no library directory of the core holds"

finish
