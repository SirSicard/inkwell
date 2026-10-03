#!/usr/bin/env bash
# The version a Windows release builds, from its tag or from the dry run's input, and its files'
# names. Prints `version=X.Y.Z`, `setup=...`, `package=...` and `sums=...` lines (for
# $GITHUB_OUTPUT), or fails. The Windows counterpart of mac/scripts/release-version.sh, with the
# same rules:
#
#   windows/scripts/release-version.sh tag v1.2.3      a release: v1.X.Y exactly
#   windows/scripts/release-version.sh dry-run 1.2.3   the dry run: X.Y.Z
#
# A release tag also waits until every licence notice the Windows app shows that was written
# without its upstream file on hand has been compared with it (mac/scripts/notices-verified.sh,
# over the Rust overrides, those of the crates in Velopack's Setup.exe and Update.exe, the shared
# composed notices and the Windows-only ones), and until the core says the same version
# (mac/scripts/core-version.sh: About shows it), as the Mac's tag does; a dry run reports either,
# on stderr, and goes on. INK_CORE_MANIFEST replaces core/Cargo.toml, for the tests.
#
# Only plain X.Y.Z, no suffix: Velopack compares versions to decide what is newer, and the Mac's
# release of the same tag allows no more. No leading zeros: "1.02" and "1.2" would name one
# version. INK_NOTICES_FILES (colon-separated) replaces the notice lists, for the tests.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
fail() { echo "release-version: $*" >&2; exit 1; }
[ $# -eq 2 ] || fail "usage: release-version.sh tag v1.X.Y | dry-run X.Y.Z"

number='(0|[1-9][0-9]{0,5})'
case "$1" in
  tag)
    [[ "$2" =~ ^v(1\.$number\.$number)$ ]] || fail "a Windows release tag is v1.X.Y, not '$2'"
    version="${BASH_REMATCH[1]}"
    ;;
  dry-run)
    [[ "$2" =~ ^($number\.$number\.$number)$ ]] || fail "a dry run's version is X.Y.Z, not '$2'"
    version="${BASH_REMATCH[1]}"
    ;;
  *) fail "unknown kind: $1" ;;
esac
lists="${INK_NOTICES_FILES:-$root/core/crates/ink-ffi/notices/overrides.txt:$root/core/crates/ink-ffi/notices/velopack/overrides.txt:$root/mac/composed-notices.txt:$root/windows/Inkwell.Core/Screens/About/composed-notices.txt}"
# stdout is the workflow's outputs: notices-verified.sh prints only to stderr.
if [ "$1" = tag ]; then
  INK_NOTICES_FILES="$lists" /bin/bash "$root/mac/scripts/notices-verified.sh" \
    || fail "a release tag needs every licence notice compared with its upstream file first (above)"
  /bin/bash "$root/mac/scripts/core-version.sh" "$version" \
    || fail "a release tag needs the core at the release's version first (above)"
else
  INK_NOTICES_FILES="$lists" /bin/bash "$root/mac/scripts/notices-verified.sh" --warn
  /bin/bash "$root/mac/scripts/core-version.sh" --warn "$version"
fi
echo "version=$version"
# x64 only (Directory.Build.props). windows/scripts/pack.ps1 writes these names.
echo "setup=Inkwell_${version}_x64-setup.exe"
echo "package=InkwellApp-${version}-full.nupkg"
echo "sums=Inkwell_${version}_windows-sha256.txt"
