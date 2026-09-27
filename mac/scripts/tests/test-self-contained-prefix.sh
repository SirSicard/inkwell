#!/usr/bin/env bash
# make_self_contained (core/crates/ink-engines/native/lib/self-contained-prefix.sh), which
# build-nemo-speech.sh runs on NeMo-Speech.cpp's install prefix before the app bundles it. Tiny
# libraries compiled here with clang stand in for NeMo's and Homebrew's; the one program run is
# built here too. Offline.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/assert.sh"
library="$(cd "$here/../../../core/crates/ink-engines/native/lib" && pwd)/self-contained-prefix.sh"

# The real path: the walk records each copy's source as realpath gives it (/var is a link).
work="$(cd "$(mktemp -d "${TMPDIR:-/tmp}/ink-self-contained-test.XXXXXX")" && pwd -P)"
trap 'rm -rf "$work"' EXIT

cc() { xcrun clang -mmacosx-version-min=26.0 "$@"; }

# fixture <dir>: a prefix whose own libraries, as a build installed them, load a chain from
# "Homebrew" (absolute install names and an absolute LC_RPATH, as Homebrew's are):
#   prefix/lib/libn.dylib   @rpath/libn.dylib; loads A by absolute path      n() = a() = 6
#   prefix/lib/libn2.dylib  @rpath/libn2.dylib; loads N by @rpath, its sibling in the prefix
#   brew/a/lib/liba.dylib   loads B by absolute path
#   brew/b/lib/libb.dylib   loads C as @rpath/libc.dylib, found through its LC_RPATH
#   brew/c/lib/libc.dylib   @rpath/libc.dylib; loads A by absolute path (a cycle), and libz from
#                           the OS
fixture() {
  local d=$1
  mkdir -p "$d/src" "$d/prefix/lib" "$d/brew/a/lib" "$d/brew/b/lib" "$d/brew/c/lib"
  echo 'int c(void) { return 3; }' >"$d/src/c0.c"
  printf '%s\n' '#include <zlib.h>' 'int a_id(void);' 'int c(void) { return 3; }' \
    'int c_cycle(void) { return a_id() + (zlibVersion() != 0); }' >"$d/src/c.c"
  echo 'int c(void); int b(void) { return 2 + c(); }' >"$d/src/b.c"
  echo 'int b(void); int a_id(void) { return 1; } int a(void) { return a_id() + b(); }' >"$d/src/a.c"
  echo 'int a(void); int n(void) { return a(); }' >"$d/src/n.c"
  echo 'int n(void); int n2(void) { return n(); }' >"$d/src/n2.c"
  echo 'int n(void); int main(void) { return n() == 6 ? 0 : 1; }' >"$d/src/main.c"
  # C first without the cycle, so B and A can link; then C again, loading A.
  cc -dynamiclib -install_name @rpath/libc.dylib -o "$d/brew/c/lib/libc.dylib" "$d/src/c0.c"
  cc -dynamiclib -install_name "$d/brew/b/lib/libb.dylib" -o "$d/brew/b/lib/libb.dylib" "$d/src/b.c" \
    "$d/brew/c/lib/libc.dylib" -Wl,-rpath,"$d/brew/c/lib"
  cc -dynamiclib -install_name "$d/brew/a/lib/liba.dylib" -o "$d/brew/a/lib/liba.dylib" "$d/src/a.c" \
    "$d/brew/b/lib/libb.dylib"
  cc -dynamiclib -install_name @rpath/libc.dylib -o "$d/brew/c/lib/libc.dylib" "$d/src/c.c" \
    "$d/brew/a/lib/liba.dylib" -lz
  cc -dynamiclib -install_name @rpath/libn.dylib -o "$d/prefix/lib/libn.dylib" "$d/src/n.c" \
    "$d/brew/a/lib/liba.dylib"
  cc -dynamiclib -install_name @rpath/libn2.dylib -o "$d/prefix/lib/libn2.dylib" "$d/src/n2.c" \
    "$d/prefix/lib/libn.dylib" -Wl,-rpath,@loader_path
  cc -o "$d/main" "$d/src/main.c" "$d/prefix/lib/libn.dylib" -Wl,-rpath,"$d/prefix/lib"
}

# walk <dir> <deployment target>: make_self_contained on the fixture's prefix, in its own bash (as
# the build script runs it, under set -e), stopped after 60 s: a walk that loops never returns.
# Sets $status and $out (what it printed, then the origins it recorded).
walk() {
  local d=$1 target=$2 pid waited=0
  status=0
  /bin/bash -c 'set -euo pipefail; . "$1"; lib="$2"; target="$3"; shift 3
    make_self_contained "$lib" "$target" "$@"
    for o in ${origins[@]+"${origins[@]}"}; do echo "origin $o"; done' _ \
    "$library" "$d/prefix/lib" "$target" "$d/prefix/lib/libn.dylib" "$d/prefix/lib/libn2.dylib" \
    >"$d/walk.log" 2>&1 &
  pid=$!
  while kill -0 "$pid" 2>/dev/null && [ "$waited" -lt 600 ]; do
    sleep 0.1
    waited=$((waited + 1))
  done
  if kill -0 "$pid" 2>/dev/null; then
    kill "$pid"
    flunk "the walk on $d did not end within 60 s"
  fi
  wait "$pid" || status=$?
  out="$(cat "$d/walk.log")"
}

loads() { otool -L "$1" | sed -nE '2,$ s/^[[:space:]]+([^ ]+) \(.*/\1/p'; }
rpaths() { otool -l "$1" | awk '$1 == "cmd" && $2 == "LC_RPATH" { getline; getline; print $2 }' | tr '\n' ' '; }
install_name() { otool -D "$1" | sed -n '2p'; }

d="$work/chain"
fixture "$d"
walk "$d" 26.0
assert_status "the walk ends and succeeds (A -> B -> C -> A is a cycle)" 0 "$status"
assert_absent "no warning: everything is built for the target" "$out" "warning"
for name in liba libb libc; do
  [ -f "$d/prefix/lib/$name.dylib" ] && pass "$name is copied into the prefix" || flunk "$name is not in the prefix"
done
assert_contains "A's origin is recorded" "$out" "origin liba.dylib $d/brew/a/lib/liba.dylib"
assert_contains "B's origin is recorded" "$out" "origin libb.dylib $d/brew/b/lib/libb.dylib"
assert_contains "C's origin is recorded" "$out" "origin libc.dylib $d/brew/c/lib/libc.dylib"
assert_status "each copied once, the sibling N never" 3 "$(grep -c '^origin ' <<<"$out")"
for name in libn libn2 liba libb libc; do
  f="$d/prefix/lib/$name.dylib"
  assert_status "$name: @loader_path is its only rpath" "@loader_path " "$(rpaths "$f")"
  assert_absent "$name: loads nothing by an absolute path outside the OS" "$(loads "$f")" "$d"
  assert_contains "$name: still loads the OS by its own path" "$(loads "$f")" "/usr/lib/libSystem.B.dylib"
  codesign --verify "$f" 2>/dev/null && pass "$name: its signature verifies" || flunk "$name: its signature does not verify"
done
for name in liba libb libc; do
  assert_status "$name: its install name is @rpath" "@rpath/$name.dylib" "$(install_name "$d/prefix/lib/$name.dylib")"
done
assert_status "N keeps its own install name" "@rpath/libn.dylib" "$(install_name "$d/prefix/lib/libn.dylib")"
assert_contains "N loads A by @rpath now" "$(loads "$d/prefix/lib/libn.dylib")" "@rpath/liba.dylib"
assert_contains "A loads B by @rpath now" "$(loads "$d/prefix/lib/liba.dylib")" "@rpath/libb.dylib"
assert_contains "B loads C by @rpath" "$(loads "$d/prefix/lib/libb.dylib")" "@rpath/libc.dylib"
assert_contains "C loads A by @rpath now" "$(loads "$d/prefix/lib/libc.dylib")" "@rpath/liba.dylib"
assert_contains "C still loads libz from the OS" "$(loads "$d/prefix/lib/libc.dylib")" "/usr/lib/libz.1.dylib"
# The proof: with "Homebrew" gone, a program linked against the prefix still loads and runs.
mv "$d/brew" "$d/brew.gone"
run_status=0
"$d/main" || run_status=$?
assert_status "the prefix loads and runs with the originals gone" 0 "$run_status"

# Again, on the finished prefix: nothing left to copy; and a target older than the libraries only
# warns (the prefix still serves this Mac; build-mac.sh refuses to ship it).
walk "$d" 15.0
assert_status "a second walk on a finished prefix" 0 "$status"
assert_status "... copies nothing" 0 "$(grep -c '^origin ' <<<"$out" || true)"
assert_contains "... and warns of code newer than the target" "$out" "warning: liba.dylib is built for macOS 26.0, newer than 15.0"

d="$work/missing"
fixture "$d"
rm "$d/brew/c/lib/libc.dylib"
walk "$d" 26.0
[ "$status" != 0 ] && pass "a dependency that is not there stops the walk" || flunk "a missing dependency passed"
assert_contains "... and says which" "$out" "libb.dylib loads @rpath/libc.dylib, which is not found"

finish
