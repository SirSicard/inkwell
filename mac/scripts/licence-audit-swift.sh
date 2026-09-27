#!/usr/bin/env bash
# The Swift licence audit (invariant I2, with cargo deny for the core). Fails on any Swift package
# dependency, direct or not, that is not on the vetted list below with an allowed licence.
#
# Agent-assisted development is how copyleft leaks in, and the GitHub API's licence field is no
# check: an attractive GPL-3.0 Swift codebase in this space reports NOASSERTION there. Discipline
# is not a control; CI is. So a package passes only when a person has read its LICENSE file and
# listed it here, and, once it is checked out, only when that file still says what was vetted.
#
#   mac/scripts/licence-audit-swift.sh              every dependency Package.swift declares must be
#                                                   pinned in the committed mac/Package.resolved (or
#                                                   be a vetted path dependency), and every pin
#                                                   vetted; LICENSE files too where checked out.
#                                                   Offline, and meant to run BEFORE any build: a
#                                                   build resolves, and what it resolved is then
#                                                   what ships (build-mac.sh builds only from the
#                                                   pinned versions).
#   mac/scripts/licence-audit-swift.sh --checkouts  after a build or `swift package resolve`: also
#                                                   require every pin's checkout and audit it, and
#                                                   every downloaded binary artifact (CI)
#
# A prebuilt binary the package itself declares (`.binaryTarget(url:checksum:)`, Sparkle) is vetted
# by its URL and checksum, before anything downloads it, and its unpacked licence file afterwards.
#
# Allowed: MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Zlib.
set -euo pipefail

mac="$(cd "$(dirname "$0")/.." && pwd)"
resolved="$mac/Package.resolved"
checkouts="$mac/.build/checkouts"
artifacts="$mac/.build/artifacts"
require_checkouts=0
for arg in "$@"; do
  case "$arg" in
    --checkouts) require_checkouts=1 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

allowed=" MIT Apache-2.0 BSD-2-Clause BSD-3-Clause ISC Zlib "

# Vetted by reading the LICENSE file, not the API field. "identity|SPDX|why". A package that is
# not here fails the audit: adding one is a deliberate act, recorded here and in THIRD_PARTY.md.
vetted=(
  "fluidaudio|Apache-2.0|Parakeet live partials on the Neural Engine (AppleEngines)"
)

# Prebuilt binaries a package downloads (remote binaryTarget): "identity/target|SPDX|why". A
# binary brings its own dependencies, so it is vetted on its own, never through its package.
vetted_binaries=(
)

# Prebuilt binaries this package itself declares (`.binaryTarget(url:checksum:)`):
# "target|SPDX|url|checksum|why". The URL names the release, and SwiftPM refuses an archive whose
# SHA-256 is not the checksum, so the two pin exactly the bytes that were vetted: a manifest that
# changes either fails here until this entry is changed too, deliberately.
vetted_own_binaries=(
  "Sparkle|MIT|https://github.com/sparkle-project/Sparkle/releases/download/2.10.0/Sparkle-for-Swift-Package-Manager.zip|17e28312b8e18ab7cdbbe09a6fb28cc55a5479ec6c371dbc07cdecd2a14fd959|in-app updates (Inkwell), Sparkle 2.10.0; its LICENSE also covers bsdiff (BSD-2-Clause), sais-lite (MIT), ed25519 (Zlib) and SUSignatureVerifier (BSD-2-Clause), all allowed; the BSD notices ship in About (THIRD_PARTY.md)"
)

# Licence files inside a package that may mention a copyleft licence without being one:
# "identity|path|why".
vetted_mentions=(
)

# Path dependencies (`.package(path:)`): never pinned, so nothing but this list vets them.
# "identity|SPDX|why"; the licence file at the path must still read as SPDX.
vetted_local=(
)

fail=0
pass() { printf 'PASS  %s\n' "$*"; }
flag() { printf 'FAIL  %s\n' "$*"; fail=1; }
note() { printf '      %s\n' "$*"; }

field() { # entry index
  cut -d'|' -f"$2" <<<"$1"
}

lookup() { # key entry... -> the entry whose first field is key, or nothing
  local key="$1" entry
  shift
  for entry in "$@"; do
    [ "$(field "$entry" 1)" = "$key" ] && { printf '%s\n' "$entry"; return 0; }
  done
  return 0
}

# The licence a LICENSE file's text is, by the phrases each licence's text contains ("unknown"
# when none match). Whitespace and case are folded first.
classify() {
  local text
  text="$(tr -s '[:space:]' ' ' <"$1" | tr '[:upper:]' '[:lower:]')"
  case "$text" in
    *"gnu affero general public license"* | *"gnu general public license"* | *"gnu lesser general public license"* | *"gnu library general public license"* | *"mozilla public license"* | *"eclipse public license"* | *"server side public license"* | *"business source license"*)
      echo copyleft ;;
    *"apache license"*"version 2.0"*) echo Apache-2.0 ;;
    *"permission is hereby granted, free of charge, to any person obtaining a copy"*) echo MIT ;;
    *"redistribution and use in source and binary forms"*)
      case "$text" in
        *"neither the name"* | *"names of its contributors"* | *"name of the copyright holder"*) echo BSD-3-Clause ;;
        *) echo BSD-2-Clause ;;
      esac
      ;;
    *"permission to use, copy, modify, and"*"distribute this software for any purpose with or without fee is hereby granted"*) echo ISC ;;
    *"provided 'as-is', without any express or implied warranty"*"altered source versions must be plainly marked"*) echo Zlib ;;
    *) echo unknown ;;
  esac
}

# The top-level licence file of a checkout: a LICENSE (or LICENCE) file first, COPYING only
# without one.
licence_file() {
  local found
  found="$(find "$1" -maxdepth 1 -type f \( -iname 'LICENSE*' -o -iname 'LICENCE*' \) | sort | head -1)"
  [ -n "$found" ] || found="$(find "$1" -maxdepth 1 -type f -iname 'COPYING*' | sort | head -1)"
  printf '%s\n' "$found"
}

# A repository URL as SwiftPM compares them: case-insensitive, with or without .git or a slash.
normalise_url() {
  tr '[:upper:]' '[:lower:]' <<<"$1" | sed -E 's#/+$##; s#\.git$##'
}

# A copyleft licence's own text anywhere in a checkout's licence files (the uppercase title that
# opens the licence itself; a line that merely names one, as in a dual-licence note, does not).
copyleft_texts() {
  find "$1" -type f \( -iname '*LICENSE*' -o -iname '*LICENCE*' -o -iname 'COPYING*' -o -iname 'NOTICE*' \) \
    -not -path '*/.git/*' -print0 \
    | xargs -0 grep -lE 'GNU (AFFERO |LESSER |LIBRARY )?GENERAL PUBLIC LICENSE|Mozilla Public License Version|Eclipse Public License|Server Side Public License|Business Source License' 2>/dev/null || true
}

echo "== Swift licence audit (mac/) =="

# --- declared against pinned -----------------------------------------------------------------------
# kind<TAB>identity<TAB>location for every dependency Package.swift declares: sourceControl (a
# URL), fileSystem (a path) or registry. Read from the manifest itself, so nothing a build could
# resolve escapes the audit.
dump="$(swift package dump-package --package-path "$mac")" || { echo "FAIL  Package.swift could not be read"; exit 1; }
declared="$(python3 -c 'import json, sys
for dep in json.load(sys.stdin)["dependencies"]:
    for kind, specs in dep.items():
        spec = specs[0]
        if kind == "sourceControl":
            remote = spec.get("location", {}).get("remote", [{}])
            where = remote[0].get("urlString", "") if remote else ""
            where = where or spec.get("location", {}).get("local", [""])[0]
        elif kind == "fileSystem":
            where = spec.get("path", "")
        else:
            where = spec.get("identity", "")
        print(kind + "\t" + spec.get("identity", "?").lower() + "\t" + where)' <<<"$dump")"

# name<TAB>url<TAB>checksum for every remote binary target the manifest itself declares (a path
# binary target, the core's InkCore, is built here from source).
own_binaries="$(python3 -c 'import json, sys
for target in json.load(sys.stdin)["targets"]:
    if target.get("type") == "binary" and target.get("url"):
        print(target["name"] + "\t" + target["url"] + "\t" + (target.get("checksum") or ""))' <<<"$dump")"
# SwiftPM files the root package's artifacts under its identity: the directory's name, lowercased.
root_identity="$(basename "$mac" | tr '[:upper:]' '[:lower:]')"

# identity<TAB>location, one per pin (Package.resolved v2 and v3).
pins=""
if [ -f "$resolved" ]; then
  pins="$(python3 -c 'import json, sys
for p in json.load(open(sys.argv[1])).get("pins", []):
    print(p.get("identity", "").lower() + "\t" + p.get("location", ""))' "$resolved")"
fi
if [ -z "$pins" ] && [ -z "$declared" ]; then
  echo "PASS  no Swift package dependencies"
fi

# --- the package's own prebuilt binaries -------------------------------------------------------------
while IFS=$'\t' read -r name url checksum; do
  [ -n "$name" ] || continue
  entry="$(lookup "$name" ${vetted_own_binaries[@]+"${vetted_own_binaries[@]}"})"
  if [ -z "$entry" ]; then
    flag "binary target $name is not on the vetted list ($url)"
    note "a prebuilt binary brings its own dependencies: read its licence, then pin its URL and"
    note "checksum in vetted_own_binaries and add it to THIRD_PARTY.md"
    continue
  fi
  spdx="$(field "$entry" 2)"
  case "$allowed" in
    *" $spdx "*) ;;
    *) flag "binary target $name is vetted as $spdx, which is not allowed"; continue ;;
  esac
  if [ "$url" != "$(field "$entry" 3)" ]; then
    flag "binary target $name: the URL is not the vetted one ($url)"
    continue
  fi
  if [ "$checksum" != "$(field "$entry" 4)" ]; then
    flag "binary target $name: the checksum is not the vetted one"
    continue
  fi
  pass "binary target $name ($spdx, vetted by URL and checksum)"
  if [ "$require_checkouts" = 1 ] && [ ! -d "$artifacts/$root_identity/$name" ]; then
    flag "binary target $name was not downloaded (no .build/artifacts/$root_identity/$name): resolve or build first"
  fi
done <<<"$own_binaries"
if [ -n "$declared" ] && grep -qv '^fileSystem' <<<"$declared" && [ ! -f "$resolved" ]; then
  flag "Package.swift declares dependencies but mac/Package.resolved is missing: resolve and commit it,"
  note "so what CI audits is exactly what ships"
fi

while IFS=$'\t' read -r kind identity where; do
  [ -n "$kind" ] || continue
  case "$kind" in
    fileSystem)
      entry="$(lookup "$identity" ${vetted_local[@]+"${vetted_local[@]}"})"
      if [ -z "$entry" ]; then
        flag "$identity is a path dependency ($where) that is not on the vetted list"
        note "a path dependency is never pinned: vet it by reading its licence, and add it to vetted_local"
        continue
      fi
      spdx="$(field "$entry" 2)"
      case "$allowed" in
        *" $spdx "*) ;;
        *) flag "$identity is vetted as $spdx, which is not allowed"; continue ;;
      esac
      lic="$(licence_file "$where")"
      if [ -z "$lic" ]; then
        flag "$identity ($where) has no licence file: all rights reserved"
        continue
      fi
      found="$(classify "$lic")"
      if [ "$found" != "$spdx" ]; then
        flag "$identity: ${lic#"$where/"} reads as $found, vetted as $spdx"
        continue
      fi
      pass "$identity ($spdx, path dependency, ${lic#"$where/"} read)"
      ;;
    *)
      pin="$(awk -F'\t' -v id="$identity" '$1 == id { print $2; exit }' <<<"$pins")"
      if [ -z "$pin" ]; then
        # Without a Package.resolved at all, that was reported above.
        if [ -f "$resolved" ]; then
          flag "$identity is declared in Package.swift but not pinned in Package.resolved:"
          note "resolve, audit the new pins, and commit Package.resolved with the change"
        fi
        continue
      fi
      if [ "$kind" = sourceControl ] && [ "$(normalise_url "$pin")" != "$(normalise_url "$where")" ]; then
        flag "$identity is declared from $where but is pinned from another location ($pin)"
      fi
      ;;
  esac
done <<<"$declared"

# --- pins ------------------------------------------------------------------------------------------
while IFS=$'\t' read -r identity location; do
  [ -n "$identity" ] || continue
  entry="$(lookup "$identity" ${vetted[@]+"${vetted[@]}"})"
  if [ -z "$entry" ]; then
    flag "$identity ($location) is not on the vetted list"
    note "read its LICENSE file (never the API field), then add it to this script and THIRD_PARTY.md"
    continue
  fi
  spdx="$(field "$entry" 2)"
  case "$allowed" in
    *" $spdx "*) ;;
    *) flag "$identity is vetted as $spdx, which is not allowed"; continue ;;
  esac

  dir="$checkouts/$(basename "${location%.git}")"
  if [ ! -d "$dir" ]; then
    if [ "$require_checkouts" = 1 ]; then
      flag "$identity: no checkout at ${dir#"$mac/"} to read its licence from"
    else
      pass "$identity ($spdx, vetted; licence file not read: no checkout)"
    fi
    continue
  fi
  lic="$(licence_file "$dir")"
  if [ -z "$lic" ]; then
    flag "$identity has no licence file: all rights reserved"
    continue
  fi
  found="$(classify "$lic")"
  if [ "$found" != "$spdx" ]; then
    flag "$identity: ${lic#"$dir/"} reads as $found, vetted as $spdx"
    continue
  fi
  mentions_ok=1
  while IFS= read -r hit; do
    [ -n "$hit" ] || continue
    rel="${hit#"$dir/"}"
    if printf '%s\n' ${vetted_mentions[@]+"${vetted_mentions[@]}"} | grep -qF "$identity|$rel|"; then
      note "$identity: $rel names a copyleft licence (vetted)"
    else
      flag "$identity: $rel carries a copyleft licence text"
      mentions_ok=0
    fi
  done <<<"$(copyleft_texts "$dir")"
  [ "$mentions_ok" = 1 ] && pass "$identity ($spdx, ${lic#"$dir/"} read)"
done <<<"$pins"

# --- binary artifacts ----------------------------------------------------------------------------
# .build/artifacts/<package identity>/<target>: prebuilt binaries SwiftPM downloaded (remote
# binaryTarget). Local ones (the core, InkCore) are not copied there.
if [ -d "$artifacts" ]; then
  for target in "$artifacts"/*/*; do
    [ -d "$target" ] || continue
    package="$(basename "$(dirname "$target")")"
    # SwiftPM's scratch space for unpacking archives: empty directories, not artifacts.
    [ "$package" = extract ] && continue
    key="$package/$(basename "$target")"
    if [ "$package" = "$root_identity" ]; then
      # One of our own: vetted above by URL and checksum; here its unpacked licence file, when it
      # ships one, must still read as vetted, and nothing inside may carry a copyleft text.
      entry="$(lookup "$(basename "$target")" ${vetted_own_binaries[@]+"${vetted_own_binaries[@]}"})"
      if [ -z "$entry" ]; then
        flag "binary artifact $key is not on the vetted list"
        continue
      fi
      spdx="$(field "$entry" 2)"
      lic="$(licence_file "$target")"
      read_note="no licence file inside"
      if [ -n "$lic" ]; then
        found="$(classify "$lic")"
        if [ "$found" != "$spdx" ]; then
          flag "binary artifact $key: ${lic#"$target/"} reads as $found, vetted as $spdx"
          continue
        fi
        read_note="${lic#"$target/"} read"
      fi
      clean=1
      while IFS= read -r hit; do
        [ -n "$hit" ] || continue
        flag "binary artifact $key: ${hit#"$target/"} carries a copyleft licence text"
        clean=0
      done <<<"$(copyleft_texts "$target")"
      [ "$clean" = 1 ] && pass "binary artifact $key ($spdx, vetted, $read_note)"
      continue
    fi
    entry="$(lookup "$key" ${vetted_binaries[@]+"${vetted_binaries[@]}"})"
    if [ -z "$entry" ]; then
      flag "binary artifact $key is not on the vetted list"
      note "a prebuilt binary brings its own dependencies: vet it on its own, or turn it off"
      note "(for a package trait, in the dependency's traits: in Package.swift)"
    else
      pass "binary artifact $key ($(field "$entry" 2), vetted)"
    fi
  done
elif [ "$require_checkouts" = 1 ] && [ -n "$pins" ]; then
  flag "no .build/artifacts to audit: resolve or build first"
fi

echo
if [ "$fail" -ne 0 ]; then
  echo "Swift licence audit FAILED"
  exit 1
fi
echo "Swift licence audit passed"
