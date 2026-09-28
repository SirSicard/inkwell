#!/usr/bin/env bash
# safe_extract (core/crates/ink-engines/native/lib/safe-extract.sh), which
# build-sentencepiece-abseil.sh extracts its pinned tarballs with: an entry that would land outside
# the directory is refused before anything is extracted. The tarballs are crafted here with bsdtar
# (-P keeps absolute and '..' names, -s renames an entry); the hard link one with Python. Offline.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/assert.sh"
. "$here/../../../core/crates/ink-engines/native/lib/safe-extract.sh"

work="$(cd "$(mktemp -d "${TMPDIR:-/tmp}/ink-safe-extract-test.XXXXXX")" && pwd -P)"
trap 'rm -rf "$work"' EXIT

# A small tree to archive: pkg/file, and pkg/sub/inside -> ../file (a link that stays inside).
mkdir -p "$work/in/pkg/sub"
printf 'data\n' >"$work/in/pkg/file"
ln -s ../file "$work/in/pkg/sub/inside"

# craft <name> <tar arguments...>: a tarball of $work/in, made with those arguments.
craft() {
  local name=$1
  shift
  (cd "$work/in" && tar -czPf "$work/$name.tar.gz" "$@")
}
# extract <label> <expected status> <name>: sets $out; a refusal must leave nothing extracted.
extract() {
  local label=$1 want=$2 name=$3 status=0
  out="$(safe_extract "$work/$name.tar.gz" "$work/out-$name" 2>&1)" || status=$?
  assert_status "$label" "$want" "$status"
  if [ "$want" != 0 ]; then
    [ ! -e "$work/out-$name" ] && pass "$label: extracts nothing" || flunk "$label: extracted into $work/out-$name"
  fi
}

craft clean pkg
extract "a tree whose one link stays inside it" 0 clean
[ -f "$work/out-clean/pkg/file" ] && pass "... is extracted" || flunk "... pkg/file is missing"
[ "$(cat "$work/out-clean/pkg/sub/inside")" = data ] && pass "... its link works" || flunk "... its link is broken"
[ "$(stat -f %u "$work/out-clean/pkg/file")" = "$(id -u)" ] && pass "... owned by whoever extracted it" \
  || flunk "... kept the archive's owner"

craft absolute -s '|^pkg/file$|/tmp/ink-safe-extract-evil|' pkg/file
extract "an entry with an absolute name" 1 absolute
assert_contains "... says why" "$out" "/tmp/ink-safe-extract-evil, an absolute path: refused"

craft dotdot -s '|^pkg/file$|pkg/../../evil|' pkg/file
extract "an entry whose name climbs out with '..'" 1 dotdot
assert_contains "... says why" "$out" "pkg/../../evil, a path with '..': refused"

ln -s ../../../etc "$work/in/pkg/sub/out"
craft escaping pkg
rm "$work/in/pkg/sub/out"
extract "a symbolic link that leads out of the tree" 1 escaping
assert_contains "... says why" "$out" "the link pkg/sub/out points at ../../../etc, outside the tree: refused"

ln -s /etc "$work/in/pkg/abs"
craft abslink pkg
rm "$work/in/pkg/abs"
extract "a symbolic link to an absolute path" 1 abslink
assert_contains "... says why" "$out" "the link pkg/abs points at /etc, an absolute path: refused"

# bsdtar renames a hard link's target with its entry, so this one is written with Python's tarfile:
# pkg/file, then pkg/hard, a hard link to ../file (every name clean, only the target climbs out).
python3 - "$work/hardlink.tar.gz" <<'PY'
import io, sys, tarfile
with tarfile.open(sys.argv[1], "w:gz") as t:
    data = b"data\n"
    f = tarfile.TarInfo("pkg/file"); f.size = len(data); t.addfile(f, io.BytesIO(data))
    h = tarfile.TarInfo("pkg/hard"); h.type = tarfile.LNKTYPE; h.linkname = "../file"; t.addfile(h)
PY
extract "a hard link to an entry outside the tree" 1 hardlink
assert_contains "... says why" "$out" "the hard link pkg/hard points at ../file, outside the tree: refused"

# Names the line-by-line listing could misread, each in its own tarball, written with Python.
# odd <name> <kind: file or symlink> <entry name> [link target]
odd() {
  python3 - "$work/$1.tar.gz" "$2" "$3" "${4-}" <<'PY'
import io, sys, tarfile
path, kind, name, target = sys.argv[1:5]
name = name.encode().decode("unicode_escape")
target = target.encode().decode("unicode_escape")
with tarfile.open(path, "w:gz") as t:
    data = b"data\n"
    f = tarfile.TarInfo("pkg/file"); f.size = len(data); t.addfile(f, io.BytesIO(data))
    if kind == "file":
        e = tarfile.TarInfo(name); e.size = len(data); t.addfile(e, io.BytesIO(data))
    else:
        e = tarfile.TarInfo(name); e.type = tarfile.SYMTYPE; e.linkname = target; t.addfile(e)
PY
}
odd newline file 'pkg/a\nb'
extract "a name with a newline" 1 newline
assert_contains "... says why" "$out" "a control character or a backslash: refused"
odd tab file 'pkg/a\tb'
extract "a name with a tab" 1 tab
assert_contains "... says why" "$out" "a control character or a backslash: refused"
odd escape file 'pkg/a\x1bb'
extract "a name with an escape character" 1 escape
assert_contains "... says why" "$out" "a control character or a backslash: refused"
odd backslash file 'pkg/a\\b'
extract "a name with a backslash" 1 backslash
assert_contains "... says why" "$out" "a control character or a backslash: refused"
odd arrow file 'pkg/a -> b'
extract "a name containing ' -> '" 1 arrow
assert_contains "... says why" "$out" "holds pkg/a -> b, a name its listing cannot be split around: refused"
odd linkto file 'pkg/a link to b'
extract "a name containing ' link to '" 1 linkto
assert_contains "... says why" "$out" "holds pkg/a link to b, a name its listing cannot be split around: refused"
odd targetnl symlink 'pkg/sl' 'file\n/etc'
extract "a link target with a newline" 1 targetnl
assert_contains "... says why" "$out" "a control character or a backslash: refused"
# The target is read whole, from its own entry's " -> ": split at the last one, this would read "y".
odd targetarrow symlink 'pkg/sl' '../../x -> y'
extract "a link target containing ' -> ' that leads out" 1 targetarrow
assert_contains "... says why" "$out" "the link pkg/sl points at ../../x -> y, outside the tree: refused"
odd fine symlink 'pkg/sl' 'file'
extract "a well-formed link written the same way" 0 fine

finish
