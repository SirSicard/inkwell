#!/usr/bin/env bash
# licence-audit-swift.sh on fixture packages: what it must pass, and every way it must fail.
# Offline: `swift package dump-package` reads a manifest without fetching anything.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/assert.sh"
audit="$here/../licence-audit-swift.sh"

work="$(mktemp -d "${TMPDIR:-/tmp}/ink-audit-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT

fluid_url="https://github.com/FluidInference/FluidAudio.git"
apache="                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/"
gpl="                    GNU GENERAL PUBLIC LICENSE
                       Version 3, 29 June 2007"
mit="MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy of this software"

# fixture <name> <dependencies (Swift)> <target dependencies (Swift)>: a package at
# $work/<name>/mac with the audit script copied in, and $pkg set to it.
fixture() {
  pkg="$work/$1/mac"
  mkdir -p "$pkg/scripts" "$pkg/Sources/App"
  cp "$audit" "$pkg/scripts/"
  echo 'print(1)' >"$pkg/Sources/App/main.swift"
  cat >"$pkg/Package.swift" <<SWIFT
// swift-tools-version: 6.2
import PackageDescription
let package = Package(name: "Fixture", platforms: [.macOS(.v26)],
  dependencies: [$2],
  targets: [.executableTarget(name: "App", dependencies: [$3])])
SWIFT
}

# pins <identity|location>...: a Package.resolved (v3) with these pins.
pins() {
  local entries="" pin
  for pin in "$@"; do
    [ -n "$entries" ] && entries="$entries,"
    entries="$entries{\"identity\":\"${pin%%|*}\",\"kind\":\"remoteSourceControl\",\"location\":\"${pin#*|}\",\"state\":{\"revision\":\"0000000000000000000000000000000000000000\",\"version\":\"0.17.4\"}}"
  done
  printf '{"originHash":"fixture","pins":[%s],"version":3}\n' "$entries" >"$pkg/Package.resolved"
}

# A local package next to the fixture, with a licence file.
local_package() { # licence text
  mkdir -p "$pkg/../LocalKit/Sources/LocalKit"
  echo 'public let x = 1' >"$pkg/../LocalKit/Sources/LocalKit/x.swift"
  printf '%s\n' "$1" >"$pkg/../LocalKit/LICENSE"
  cat >"$pkg/../LocalKit/Package.swift" <<'SWIFT'
// swift-tools-version: 6.2
import PackageDescription
let package = Package(name: "LocalKit", products: [.library(name: "LocalKit", targets: ["LocalKit"])],
  targets: [.target(name: "LocalKit")])
SWIFT
}

# A checkout of FluidAudio as `swift build` leaves it (with its artifacts directory).
checkout() { # licence text
  mkdir -p "$pkg/.build/checkouts/FluidAudio" "$pkg/.build/artifacts"
  printf '%s\n' "$1" >"$pkg/.build/checkouts/FluidAudio/LICENSE"
}

# run <label> <expected status> <expected text> [audit arguments]
run() {
  local label=$1 want=$2 text=$3 out status=0
  shift 3
  out="$(/bin/bash "$pkg/scripts/licence-audit-swift.sh" "$@" 2>&1)" || status=$?
  assert_status "$label" "$want" "$status"
  assert_contains "$label: says why" "$out" "$text"
}

fluid_dep=".package(url: \"$fluid_url\", from: \"0.17.0\", traits: [])"
fluid_use='.product(name: "FluidAudio", package: "FluidAudio")'

fixture none "" ""
run "no dependencies" 0 "no Swift package dependencies"

fixture pinned "$fluid_dep" "$fluid_use"
pins "fluidaudio|$fluid_url"
run "declared, pinned and vetted" 0 "PASS  fluidaudio"
checkout "$apache"
run "... and its checkout's licence read" 0 "LICENSE read" --checkouts

fixture unpinned "$fluid_dep" "$fluid_use"
pins
run "declared but not pinned" 1 "fluidaudio is declared in Package.swift but not pinned"

fixture unresolved "$fluid_dep" "$fluid_use"
run "declared, no Package.resolved" 1 "mac/Package.resolved is missing"

fixture moved "$fluid_dep" "$fluid_use"
pins "fluidaudio|https://example.invalid/someone-else/FluidAudio.git"
run "pinned from another location" 1 "is pinned from another location"

fixture unvetted-pin "$fluid_dep" "$fluid_use"
pins "fluidaudio|$fluid_url" "voiceink|https://example.invalid/VoiceInk.git"
run "a pin nobody vetted" 1 "voiceink (https://example.invalid/VoiceInk.git) is not on the vetted list"

fixture local ".package(path: \"../LocalKit\")" '"LocalKit"'
local_package "$mit"
run "an unvetted path dependency" 1 "localkit is a path dependency"

fixture local-vetted ".package(path: \"../LocalKit\")" '"LocalKit"'
local_package "$mit"
sed -i '' 's/^vetted_local=($/vetted_local=(\
  "localkit|MIT|a fixture"/' "$pkg/scripts/licence-audit-swift.sh"
run "a vetted path dependency" 0 "PASS  localkit (MIT, path dependency"

fixture local-relicensed ".package(path: \"../LocalKit\")" '"LocalKit"'
local_package "$gpl"
sed -i '' 's/^vetted_local=($/vetted_local=(\
  "localkit|MIT|a fixture"/' "$pkg/scripts/licence-audit-swift.sh"
run "a vetted path dependency whose licence changed" 1 "reads as copyleft, vetted as MIT"

fixture gpl-inside "$fluid_dep" "$fluid_use"
pins "fluidaudio|$fluid_url"
checkout "$apache"
printf '%s\n' "$gpl" >"$pkg/.build/checkouts/FluidAudio/COPYING"
run "a copyleft text inside a checkout" 1 "COPYING carries a copyleft licence text"

fixture mismatch "$fluid_dep" "$fluid_use"
pins "fluidaudio|$fluid_url"
checkout "$mit"
run "a licence file that is not the vetted licence" 1 "LICENSE reads as MIT, vetted as Apache-2.0"

fixture binary "$fluid_dep" "$fluid_use"
pins "fluidaudio|$fluid_url"
checkout "$apache"
mkdir -p "$pkg/.build/artifacts/fluidaudio/NemoTextProcessing"
run "an unvetted prebuilt binary" 1 "binary artifact fluidaudio/NemoTextProcessing is not on the vetted list" --checkouts

finish
