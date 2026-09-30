#!/usr/bin/env bash
# The version a Windows release builds, from its tag or from the dry run's input, and one
# architecture's file names. Prints `version=X.Y.Z`, `setup=...`, `package=...`, `feed=...` and
# `sums=...` lines (for $GITHUB_OUTPUT), or fails. The Windows counterpart of
# mac/scripts/release-version.sh, with the same rules:
#
#   windows/scripts/release-version.sh tag v1.2.3 x64        a release: v1.X.Y exactly
#   windows/scripts/release-version.sh dry-run 1.2.3 arm64   the dry run: X.Y.Z
#
# The architecture is x64 or arm64: each has its own installer and its own update channel
# (windows/scripts/pack.ps1 writes these names), and both go on the same release.
#
# A release tag also waits until every licence notice the Windows app shows that was written
# without its upstream file on hand has been compared with it (mac/scripts/notices-verified.sh,
# over the Rust overrides, the shared composed notices and the Windows-only ones); a dry run lists
# those still open, on stderr, and goes on.
#
# Only plain X.Y.Z, no suffix: Velopack compares versions to decide what is newer, and the Mac's
# release of the same tag allows no more. No leading zeros: "1.02" and "1.2" would name one
# version. INK_NOTICES_FILES (colon-separated) replaces the notice lists, for the tests.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
fail() { echo "release-version: $*" >&2; exit 1; }
[ $# -eq 3 ] || fail "usage: release-version.sh (tag v1.X.Y | dry-run X.Y.Z) (x64 | arm64)"

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
case "$3" in
  x64) channel=win ;;
  arm64) channel=win-arm64 ;;
  *) fail "the architecture is x64 or arm64, not '$3'" ;;
esac
lists="${INK_NOTICES_FILES:-$root/core/crates/ink-ffi/notices/overrides.txt:$root/mac/composed-notices.txt:$root/windows/Inkwell.Core/Screens/About/composed-notices.txt}"
# stdout is the workflow's outputs: notices-verified.sh prints only to stderr.
if [ "$1" = tag ]; then
  INK_NOTICES_FILES="$lists" /bin/bash "$root/mac/scripts/notices-verified.sh" \
    || fail "a release tag needs every licence notice compared with its upstream file first (above)"
else
  INK_NOTICES_FILES="$lists" /bin/bash "$root/mac/scripts/notices-verified.sh" --warn
fi
echo "version=$version"
# windows/scripts/pack.ps1 writes these names. Velopack names a package of Windows' default channel
# (win, x64's) without the channel, and any other channel's with it.
echo "setup=Inkwell_${version}_$3-setup.exe"
if [ "$channel" = win ]; then
  echo "package=InkwellApp-${version}-full.nupkg"
else
  echo "package=InkwellApp-${version}-${channel}-full.nupkg"
fi
echo "feed=releases.${channel}.json"
echo "sums=Inkwell_${version}_windows-$3-sha256.txt"
