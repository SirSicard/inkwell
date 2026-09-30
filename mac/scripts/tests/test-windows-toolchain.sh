#!/usr/bin/env bash
# windows_toolchain (core/crates/ink-engines/native/lib/windows-toolchain.sh), which
# build-sentencepiece-abseil.sh and build-nemo-speech.sh read their Windows architecture and
# compiler from: cl on x64, clang-cl targeting aarch64 on arm64, anything else refused. Stub
# compilers stand in for Visual Studio's and LLVM's. Offline.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/assert.sh"
library="$(cd "$here/../../../core/crates/ink-engines/native/lib" && pwd)/windows-toolchain.sh"

work="$(cd "$(mktemp -d "${TMPDIR:-/tmp}/ink-windows-toolchain-test.XXXXXX")" && pwd -P)"
trap 'rm -rf "$work"' EXIT

# Stub directories: none; cl alone; clang-cl targeting aarch64 (printing CRLF line ends, as a
# Windows program may); clang-cl targeting x86_64 (an x64 LLVM).
mkdir -p "$work/none" "$work/cl" "$work/arm" "$work/x86"
printf '#!/bin/sh\nexit 0\n' >"$work/cl/cl"
printf '#!/bin/sh\nprintf "clang version 23.1.2\\r\\nTarget: aarch64-pc-windows-msvc\\r\\nThread model: posix\\r\\n"\n' \
  >"$work/arm/clang-cl"
printf '#!/bin/sh\nprintf "clang version 23.1.2\\nTarget: x86_64-pc-windows-msvc\\nThread model: posix\\n"\n' \
  >"$work/x86/clang-cl"
chmod +x "$work/cl/cl" "$work/arm/clang-cl" "$work/x86/clang-cl"

# run <label> <expected status> <VSCMD_ARG_TGT_ARCH, or empty for unset> <stub directory>:
# windows_toolchain in its own bash, with nothing on PATH but the stubs and the system's tools.
# Sets $out: what it printed, then "<win_arch> <win_cc> <win_cc_name>" when it succeeded.
run() {
  local label=$1 want=$2 arch=$3 stubs=$4 status=0
  out="$(env -i PATH="$stubs:/usr/bin:/bin" ${arch:+VSCMD_ARG_TGT_ARCH="$arch"} /bin/bash -c \
    'set -euo pipefail; . "$1"; windows_toolchain; echo "result: $win_arch $win_cc $win_cc_name"' \
    _ "$library" 2>&1)" || status=$?
  assert_status "$label" "$want" "$status"
}

run "x64 builds with cl" 0 x64 "$work/cl"
assert_contains "... named msvc" "$out" "result: x64 cl msvc"

run "arm64 builds with clang-cl targeting aarch64" 0 arm64 "$work/arm"
assert_contains "... named clang-cl (its CRLF version output read)" "$out" "result: arm64 clang-cl clang-cl"

run "no developer environment is refused" 1 "" "$work/cl"
assert_contains "... by name" "$out" "VSCMD_ARG_TGT_ARCH is not set"

run "another target (x86) is refused" 1 x86 "$work/cl"
assert_contains "... by name" "$out" "targets 'x86'"

run "x64 without cl is refused" 1 x64 "$work/none"
assert_contains "... by name" "$out" "no cl on PATH"

run "arm64 without clang-cl is refused, even with cl" 1 arm64 "$work/cl"
assert_contains "... by name" "$out" "no clang-cl on PATH"

run "arm64 with an x64 clang-cl is refused" 1 arm64 "$work/x86"
assert_contains "... naming its target" "$out" "targets 'x86_64-pc-windows-msvc'"
assert_absent "... and sets nothing" "$out" "result:"

finish
