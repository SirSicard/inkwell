#!/usr/bin/env bash
# Fetches Sparkle's command-line tools, from the same Sparkle release as the framework
# mac/Package.swift pins, and refuses them unless the archive is exactly the one vetted: its size
# and SHA-256 are pinned here. generate_appcast and sign_update sign the appcast (the release
# workflow); generate_keys is for the maintainer, once, to make the update key (docs/RELEASING.md).
#
#   mac/scripts/sparkle-tools.sh <dir>     puts the tools in <dir>/bin
#
# Environment:
#   SPARKLE_TOOLS_ARCHIVE  a copy of the archive already on disk, checked the same way instead of
#                          downloaded (a local run without the network)
#
# This only installs the tools: no key is made or read here.
set -euo pipefail

url="https://github.com/sparkle-project/Sparkle/releases/download/2.10.0/Sparkle-2.10.0.tar.xz"
size=16319840
sha256=c2bf58aa8387266ac179357b1415d6f2635f044da8be41042af32425dae6da0c

fail() { echo "sparkle-tools: $*" >&2; exit 1; }
[ $# -eq 1 ] || fail "usage: sparkle-tools.sh <dir>"
dest="$1"
mkdir -p "$dest"
archive="$dest/$(basename "$url")"

if [ -n "${SPARKLE_TOOLS_ARCHIVE:-}" ]; then
  cp "$SPARKLE_TOOLS_ARCHIVE" "$archive"
else
  curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' --tlsv1.2 --retry 3 \
    --max-filesize $((size + 1)) --output "$archive" "$url"
fi
[ "$(stat -f %z "$archive")" = "$size" ] || fail "the archive is not $size bytes: refused"
[ "$(shasum -a 256 "$archive" | cut -d' ' -f1)" = "$sha256" ] || fail "the archive's SHA-256 is not the vetted one: refused"

tar -xJf "$archive" -C "$dest" bin/generate_appcast bin/sign_update bin/generate_keys
rm -f "$archive"
for tool in generate_appcast sign_update generate_keys; do
  [ -x "$dest/bin/$tool" ] || fail "$tool is missing from the archive"
done
echo "sparkle tools: $dest/bin (Sparkle 2.10.0, SHA-256 checked)"
