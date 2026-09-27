#!/usr/bin/env bash
# check_bundle_linkage (lib/bundle-check.sh), which build-mac.sh runs on every app it builds: a
# bundle that loads anything from outside itself and the OS, finds a library through an absolute
# rpath, cannot resolve a library it loads, or holds code built for a newer macOS than the app's
# target, fails. Tiny fixture bundles are compiled here with clang; nothing is signed or run.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/assert.sh"
. "$here/../lib/bundle-check.sh"

work="$(mktemp -d "${TMPDIR:-/tmp}/ink-bundle-check-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT

echo 'int foo(void) { return 1; }' >"$work/foo.c"
echo 'int foo(void); int main(void) { return foo() - 1; }' >"$work/main.c"

# bundle <name> <install name of libfoo> [extra flags for the executable...]: an app whose
# executable loads libfoo by that name, with libfoo in Contents/Frameworks.
bundle() {
  local name=$1 install_name=$2 app="$work/$1.app"
  shift 2
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Frameworks"
  xcrun clang -dynamiclib -mmacosx-version-min=26.0 -install_name "$install_name" \
    -o "$app/Contents/Frameworks/libfoo.dylib" "$work/foo.c"
  xcrun clang -mmacosx-version-min=26.0 -o "$app/Contents/MacOS/App" "$work/main.c" \
    "$app/Contents/Frameworks/libfoo.dylib" "$@"
}

# check <label> <expected status> <expected text or ""> <app> [target] [allow newer]
check() {
  local label=$1 want=$2 text=$3 app=$4 out status=0
  out="$(check_bundle_linkage "$app" "${5:-26.0}" "${6:-0}" 2>&1)" || status=$?
  if [ "$want" = 0 ]; then
    assert_status "$label: passes" 0 "$status"
  elif [ "$status" = 0 ]; then
    flunk "$label: it passed"
  else
    pass "$label: fails"
  fi
  if [ -n "$text" ]; then
    assert_contains "$label: says why" "$out" "$text"
  fi
}

bundle good @rpath/libfoo.dylib -Wl,-rpath,@executable_path/../Frameworks
check "a library in Frameworks, by @rpath" 0 "" "$work/good.app"

bundle homebrew /opt/homebrew/opt/foo/lib/libfoo.dylib
check "a library loaded from Homebrew" 1 "/opt/homebrew/opt/foo/lib/libfoo.dylib" "$work/homebrew.app"

bundle elsewhere /usr/local/lib/libfoo.dylib
check "a library loaded from /usr/local" 1 "/usr/local/lib/libfoo.dylib" "$work/elsewhere.app"

bundle abs-rpath @rpath/libfoo.dylib -Wl,-rpath,@executable_path/../Frameworks -Wl,-rpath,/opt/homebrew/lib
check "an absolute rpath" 1 "rpath /opt/homebrew/lib" "$work/abs-rpath.app"

bundle no-rpath @rpath/libfoo.dylib
check "an @rpath library with no rpath to find it" 1 "@rpath/libfoo.dylib" "$work/no-rpath.app"

bundle missing @rpath/libfoo.dylib -Wl,-rpath,@executable_path/../Frameworks
rm "$work/missing.app/Contents/Frameworks/libfoo.dylib"
check "an @rpath library missing from the bundle" 1 "@rpath/libfoo.dylib" "$work/missing.app"

bundle escape @rpath/libfoo.dylib -Wl,-rpath,@executable_path/../../..
cp "$work/escape.app/Contents/Frameworks/libfoo.dylib" "$work/libfoo.dylib"
check "an rpath that leads out of the bundle" 1 "@rpath/libfoo.dylib" "$work/escape.app"

check "code built for a newer macOS than the target" 1 "built for macOS 26.0" "$work/good.app" 25.0
check "the same, allowed for a local build" 0 "built for macOS 26.0" "$work/good.app" 25.0 1
check "the same, allowed, and the summary says so" 0 "2 built for a newer macOS than 25.0" "$work/good.app" 25.0 1

finish
