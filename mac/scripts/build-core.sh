#!/usr/bin/env bash
# Builds the Rust core (core/crates/ink-ffi) as a static library and wraps it, with inkwell.h and
# its module map, into mac/build/InkCore.xcframework: the InkCore binary target in Package.swift.
# Beside it, mac/build/InkCore.link: the linker arguments the app needs for this build of the core
# (one per line; build-mac.sh passes them on and bundles the libraries they name).
#
#   mac/scripts/build-core.sh                      release, engine features off (what CI builds)
#   INK_CORE_PROFILE=debug mac/scripts/build-core.sh
#   INK_CORE_FEATURES=engine-llama mac/scripts/build-core.sh
#   INK_CORE_FEATURES=engine-llama,ink-engines/engine-silero,ink-engines/engine-nemo \
#     NEMO_SPEECH_DIR=<prefix> mac/scripts/build-core.sh      the release's engines
#                                                  (build-mac.sh --engines)
#
# A feature of a crate below ink-ffi is named as <crate>/<feature>. engine-nemo links
# NeMo-Speech.cpp's library from NEMO_SPEECH_DIR (core/crates/ink-engines/native/
# build-nemo-speech.sh builds that prefix).
#
# Needs jq (/usr/bin/jq on current macOS, and on GitHub's macOS runners) for cargo's JSON messages.
#
# Extra cargo flags go in CARGO_FLAGS (for example --offline). Apple Silicon only: the one Rust
# target built here is the host's. An Intel slice needs `rustup target add x86_64-apple-darwin`,
# a second `cargo build --target x86_64-apple-darwin`, and `lipo -create` of the two archives
# before -create-xcframework (a decision for the release pipeline).
#
# Checked here, because a wrong value still builds and runs on the Mac that built it:
# - llama.cpp's ggml is configured with GGML_NATIVE=OFF (read back from its CMake cache): code
#   tuned to this Mac's CPU stops an older Apple silicon Mac with an illegal instruction. With no
#   -march either, ggml targets the compiler's arm64 macOS default, the M1's instruction set.
#   (llama-cpp-sys-2 turns native on only for RUSTFLAGS with target-cpu=native.)
# - NeMo-Speech.cpp's prefix records the same in its manifest (checked by ink-engines' build.rs).
set -euo pipefail

mac="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$mac/.." && pwd)"
profile="${INK_CORE_PROFILE:-release}"
features="${INK_CORE_FEATURES:-}"
fail() { echo "build-core: $*" >&2; exit 1; }
# Package.swift's platform. Without it the C parts of the core (SQLite, llama.cpp) are compiled for
# the building Mac's own macOS version, and the linker warns when the app targets an older one.
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-26.0}"
# Build scripts and proc macros (host code, never shipped) are not stripped. The deployment target
# above reaches them too, and on macOS 27 with Xcode 27, the release profile's strip (debuginfo,
# cargo's default) leaves a library built for macOS 26 or later with a LINKEDIT string pool that
# dyld refuses ("mis-aligned LINKEDIT string pool"): rustc then cannot load a proc macro it just
# built (rustversion, reached through llama.cpp's encoding_rs, fails as "can't find crate"). An
# unstripped one loads, as does one built for the default target. Cargo does not fingerprint the
# deployment target, so a proc macro broken that way stays cached until it is cleaned. The
# staticlib itself is never stripped.
export CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_STRIP=none

# `cargo rustc` for the staticlib alone, which is all the XCFramework takes, and so that rustc
# prints the native libraries the archive needs (native-static-libs). Cargo's JSON messages name
# each build script's output directory: llama.cpp's CMake cache is read from there.
args=(rustc -p ink-ffi --lib --crate-type staticlib --locked --message-format=json-render-diagnostics)
if [ "$profile" = "release" ]; then
  args+=(--release)
fi
if [ -n "$features" ]; then
  args+=(--features "$features")
fi
# shellcheck disable=SC2206 # CARGO_FLAGS is split on purpose
args+=(${CARGO_FLAGS:-})
args+=(-- --print=native-static-libs)
mkdir -p "$mac/build"
messages="$mac/build/core-build.json"
diagnostics="$mac/build/core-build.log"
# stderr (rustc's rendered notes, cargo's progress) is shown as it comes and kept for the note;
# stdout (the JSON messages) goes to a file.
(cd "$root/core" && cargo "${args[@]}") 2>&1 >"$messages" | tee "$diagnostics" >&2

# The native libraries, as rustc printed them. Cargo replays a fresh unit's notes, so a build with
# nothing to do prints them too.
natives="$(sed -n 's/^note: native-static-libs: //p' "$diagnostics" | tail -1)"
[ -n "$natives" ] || fail "rustc printed no native-static-libs for ink-ffi (see $diagnostics)"

# Whatever feature brought llama.cpp in, its build script ran (or was fresh) in this build.
out_dir="$(jq -r 'select(.reason == "build-script-executed" and (.package_id | test("llama-cpp-sys-2"))) | .out_dir' "$messages" | tail -1)"
if [ -n "$out_dir" ]; then
  cache="$out_dir/build/CMakeCache.txt"
  [ -f "$cache" ] || fail "llama.cpp's CMake cache is not in its build script's output (see $messages)"
  native="$(sed -n 's/^GGML_NATIVE:BOOL=//p' "$cache")"
  [ "$native" = OFF ] || fail "llama.cpp was configured with GGML_NATIVE=${native:-unset}: code for this Mac's CPU only"
  arch="$(sed -n 's/^GGML_CPU_ARM_ARCH:STRING=//p' "$cache")"
  [ -z "$arch" ] || fail "llama.cpp was configured with GGML_CPU_ARM_ARCH=$arch: the release takes the compiler's arm64 default"
  cpu=""
  for f in DOTPROD FP16_VECTOR_ARITHMETIC MATMUL_INT8 SVE SME; do
    [ "$(sed -n "s/^HAVE_$f:INTERNAL=//p" "$cache")" = 1 ] && cpu="$cpu ${f}"
  done
  echo "llama.cpp: GGML_NATIVE=OFF, no -march; ARM features compiled in:${cpu:- none}"
fi
# The core links NeMo-Speech.cpp's library (engine-nemo): the app links and bundles it from the
# prefix ink-engines' build.rs checked, NEMO_SPEECH_DIR.
nemo_lib=""
case " $natives " in
  *" -lnemo_speech_asr_c "*)
    case "${NEMO_SPEECH_DIR:-}" in /*) ;; *) fail "the core links NeMo-Speech.cpp, but NEMO_SPEECH_DIR is not an absolute path" ;; esac
    nemo_lib="$NEMO_SPEECH_DIR/lib"
    echo "NeMo-Speech.cpp: $(grep '^ggml_native ' "$NEMO_SPEECH_DIR/share/inkwell/nemo-speech.manifest")"
    ;;
esac

# The archive, where cargo says it put it (CARGO_TARGET_DIR moves it).
lib="$(jq -r 'select(.reason == "compiler-artifact" and .target.name == "ink_ffi") | .filenames[] | select(endswith(".a"))' "$messages" | tail -1)"
[ -f "$lib" ] || fail "cargo reported no ink_ffi static library (see $messages)"
headers="$mac/build/headers"
out="$mac/build/InkCore.xcframework"
link="$mac/build/InkCore.link"
rm -rf "$headers" "$out" "$link"
mkdir -p "$headers"
cp "$root/core/crates/ink-ffi/include/inkwell.h" "$root/core/crates/ink-ffi/include/module.modulemap" "$headers/"
xcodebuild -create-xcframework -library "$lib" -headers "$headers" -output "$out" >/dev/null

# One argument per line. `features` is a comment build-mac.sh reads back; the -L directory is
# where the app's bundled libraries come from.
{
  echo "# features: $features"
  [ -z "$nemo_lib" ] || echo "-L$nemo_lib"
  # shellcheck disable=SC2086 # split into words on purpose: "-framework Metal" is two arguments
  printf '%s\n' $natives
} >"$link"
echo "built $out ($profile${features:+, features: $features})"
