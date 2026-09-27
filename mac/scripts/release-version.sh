#!/usr/bin/env bash
# The version a Mac release builds, from its tag or from the dry run's input, and the dmg's name.
# Prints `version=X.Y.Z` and `dmg=...` lines (for $GITHUB_OUTPUT), or fails.
#
#   mac/scripts/release-version.sh tag v1.2.3      a release: v1.X.Y exactly
#   mac/scripts/release-version.sh dry-run 1.2.3   the dry run: X.Y.Z
#
# Only plain X.Y.Z, no suffix: the version is also the bundle's CFBundleVersion, which macOS
# requires to be at most three integers, and which Sparkle compares to decide what is newer. A
# pre-release is a dry run, not a tag. No leading zeros: "1.02" and "1.2" would name one version.
set -euo pipefail

fail() { echo "release-version: $*" >&2; exit 1; }
[ $# -eq 2 ] || fail "usage: release-version.sh tag v1.X.Y | dry-run X.Y.Z"

number='(0|[1-9][0-9]{0,5})'
case "$1" in
  tag)
    [[ "$2" =~ ^v(1\.$number\.$number)$ ]] || fail "a Mac release tag is v1.X.Y, not '$2'"
    version="${BASH_REMATCH[1]}"
    ;;
  dry-run)
    [[ "$2" =~ ^($number\.$number\.$number)$ ]] || fail "a dry run's version is X.Y.Z, not '$2'"
    version="${BASH_REMATCH[1]}"
    ;;
  *) fail "unknown kind: $1" ;;
esac
echo "version=$version"
# Apple silicon only: the core and the app are built for the runner's arm64.
echo "dmg=Inkwell_${version}_aarch64.dmg"
