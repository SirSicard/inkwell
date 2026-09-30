#!/usr/bin/env bash
# Builds and installs the NeMo-Speech.cpp diarization library that `engine-nemo` links against.
#
#   build-nemo-speech.sh <source> <prefix> [build-dir]
#
# <source>  a NeMo-Speech.cpp checkout (Apache-2.0, github.com/NVIDIA/NeMo-Speech.cpp) at the
#           pinned commit below, with its ggml submodule initialised
#           (`git submodule update --init ggml`). Nothing is downloaded here.
# <prefix>  where to install: <prefix>/lib holds libnemo_speech_asr_c, NeMo's own ggml libraries
#           and every library they load from outside the OS (on Windows <prefix>/bin holds the
#           DLLs and <prefix>/lib the C API's import library), <prefix>/include/nemo_speech the C
#           headers, and <prefix>/share/inkwell/nemo-speech.manifest the commit, the build's
#           GGML_NATIVE and each library's SHA-256. Point NEMO_SPEECH_DIR at it when building
#           ink-engines with `--features engine-nemo`.
#
# ENGINE_DEPS_DIR   the prefix build-sentencepiece-abseil.sh installed SentencePiece and Abseil to,
#                   at their pinned versions: NeMo is built against those, and every library the
#                   prefix copies in must come from there, or the script stops. A release sets it.
#                   Unset, CMake finds SentencePiece and Abseil wherever they are installed
#                   (Homebrew): fine for this Mac's tests, refused by the release's build manifest.
#
# Why a script and an environment variable, not a vendored copy or a submodule: the repository
# stays free of C++ sources and of anyone's local paths, cargo never runs CMake or reaches the
# network, and the Rust side checks what it links against: build.rs compares the installed
# headers with the pinned commit's, and every library with the manifest written below, so a
# prefix whose libraries were rebuilt or swapped after this script ran is refused. (The hashes pin
# this build, not the source: another machine's build of the same commit hashes differently.)
# NeMo is built as its own shared libraries with its own ggml,
# so it neither links nor exports symbols into the Rust binary, whatever the llama.cpp adapter
# does with its ggml.
#
# Built to ship, on every run (the app bundles this prefix's libraries; docs/ARCHITECTURE.md):
# - GGML_NATIVE=OFF, with no -march or -mcpu: ggml is compiled for the compiler's arm64 macOS
#   default (the Apple M1's instruction set), not for the building Mac's CPU. A native build on a
#   newer Mac compiles in i8mm and SME kernels that stop an older one with an illegal instruction.
# - The deployment target is MACOSX_DEPLOYMENT_TARGET, else 26.0 (mac/Package.swift's platform):
#   without it CMake targets the building Mac's own macOS, and dyld refuses the library on older
#   ones.
# - The prefix is self-contained (macOS): every library NeMo loads from outside the OS
#   (SentencePiece and Abseil, from ENGINE_DEPS_DIR) is copied into <prefix>/lib, and every library there
#   is loaded by @rpath, with `@loader_path` as its only rpath. A copy was signed by its builder;
#   after its load commands change it is signed again, ad hoc (the app re-signs it with its own
#   identity). Nothing in the prefix names Homebrew or any other absolute path outside /usr/lib
#   and /System/Library, which is checked before the manifest is written.
# - CMake is configured afresh (`--fresh`), so no cached value from an earlier configuration of
#   the same build directory (a GGML_NATIVE=ON, a CPU feature probe) carries over.
#
# Needs CMake 3.24 or later, Ninja (on macOS `brew install cmake ninja`), and SentencePiece and
# Abseil (Apache-2.0): pinned, from build-sentencepiece-abseil.sh through ENGINE_DEPS_DIR, or for a
# local build whatever is installed (`brew install sentencepiece abseil`). The configuration is the
# upstream `metal-diar` preset (Metal, standalone diarization, unpatched-ggml code paths) with the
# CLI off.
#
# On Windows (Git Bash inside a Visual Studio developer environment, with the Vulkan SDK for the
# shader compiler): the upstream `vulkan-diar` preset, built with MSVC. ENGINE_DEPS_DIR is required:
# SentencePiece and Abseil are the pinned static libraries, linked into NeMo's own DLL, so the
# prefix carries NeMo's DLLs and its ggml's and nothing else. The manifest hashes every DLL in
# <prefix>/bin and the C API's import library.
set -euo pipefail

NEMO_COMMIT=97a15afa5caa9bce5baaa86c1184103877af4101
GGML_COMMIT=c03b4e2bcece5134827881af90242086daf75be5

# make_self_contained (macOS): copies in what NeMo's libraries load from outside the OS, rewrites
# every load to @rpath with @loader_path as the only rpath, and checks the result. Its own file so
# that it can be tested without building NeMo; sourced first, so a missing copy fails before the
# build rather than after it.
. "$(cd "$(dirname "$0")" && pwd)/lib/self-contained-prefix.sh"

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    sed -n '2,20p' "$0" >&2
    exit 2
fi
src="$(cd "$1" && pwd)"
prefix="$2"
build="${3:-${src}/build/inkwell-diar}"

head="$(git -C "${src}" rev-parse HEAD)"
if [ "${head}" != "${NEMO_COMMIT}" ]; then
    echo "error: ${src} is at ${head}, not the pinned ${NEMO_COMMIT}" >&2
    exit 1
fi
ggml="$(git -C "${src}/ggml" rev-parse HEAD 2>/dev/null || true)"
if [ "${ggml}" != "${GGML_COMMIT}" ]; then
    echo "error: the ggml submodule is at '${ggml}', not the pinned ${GGML_COMMIT}" >&2
    echo "       run: git -C ${src} submodule update --init ggml" >&2
    exit 1
fi

# Upstream's own patch step: applies the in-tree ggml patch series, or confirms the complete
# series is already applied, and fails on anything else.
"${src}/scripts/apply-ggml-patches.sh"

platform=()
macos=0
windows=0
preset=metal-diar
case "$(uname -s)" in
    Darwin)
        macos=1
        deployment_target="${MACOSX_DEPLOYMENT_TARGET:-26.0}"
        platform+=("-DCMAKE_OSX_DEPLOYMENT_TARGET=${deployment_target}")
        ;;
    MINGW* | MSYS*)
        windows=1
        preset=vulkan-diar
        command -v cl >/dev/null 2>&1 || {
            echo "error: no cl on PATH: run from a Visual Studio developer environment" >&2
            exit 1
        }
        [ -n "${ENGINE_DEPS_DIR:-}" ] || {
            echo "error: set ENGINE_DEPS_DIR (build-sentencepiece-abseil.sh) on Windows" >&2
            exit 1
        }
        [ -n "${VULKAN_SDK:-}" ] || {
            echo "error: VULKAN_SDK is not set: the Vulkan backend needs the Vulkan SDK" >&2
            exit 1
        }
        platform+=(
            -DCMAKE_C_COMPILER=cl
            -DCMAKE_CXX_COMPILER=cl
            -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL
            # The Visual C++ runtime is the system's (the Rust side needs it too), not a copy in
            # the prefix under Microsoft's redistribution terms.
            -DCMAKE_INSTALL_SYSTEM_RUNTIME_LIBS_SKIP=ON
            # ggml-vulkan's find_package(SPIRV-Headers): the SDK's copy, named outright.
            "-DSPIRV-Headers_DIR=$(cygpath -m "${VULKAN_SDK}")/Lib/cmake/SPIRV-Headers"
        )
        ;;
esac
# CMake on Windows takes Windows paths (C:/...), not Git Bash's (/c/...).
native() {
    if [ "${windows}" = 1 ]; then cygpath -m "$1"; else printf '%s\n' "$1"; fi
}

# SentencePiece and Abseil: the pinned builds, named outright so that no other install is found
# first, by the names NeMo-Speech.cpp's src/asr/CMakeLists.txt reads at the pinned commit (on
# macOS: find_package(unofficial-sentencepiece) first, a vcpkg config, turned off here; then
# find_library(SENTENCEPIECE_LIB) and find_path(SENTENCEPIECE_INCLUDE_DIR), which a cached value
# skips; then find_package(absl CONFIG), from absl_DIR). What CMake used is read back from its
# cache after configuring. Their libraries load each other by @rpath, so NeMo's installed libraries
# get their directory as an rpath (CMAKE_INSTALL_RPATH_USE_LINK_PATH), by which make_self_contained
# finds them; it then leaves `@loader_path` as the only rpath.
deps=()
deps_lib=""
if [ -n "${ENGINE_DEPS_DIR:-}" ]; then
    deps_dir="$(native "$(cd "${ENGINE_DEPS_DIR}" && pwd -P)")"
    deps_lib="${deps_dir}/lib"
    if [ "${windows}" = 1 ]; then
        spm_lib=sentencepiece.lib
        # A static library carries no link dependencies, and NeMo's CMake adds only some of
        # Abseil's: the rest of what SentencePiece links (its src/CMakeLists.txt, SPM_LIBS) goes in
        # after it, as the Abseil package's targets.
        spm_absl=";absl::status;absl::status_builder;absl::strings;absl::flags;absl::flags_parse"
        spm_absl+=";absl::log;absl::log_initialize;absl::check;absl::random_random;absl::time"
    else
        spm_lib=libsentencepiece.dylib
        spm_absl=""
    fi
    for f in share/inkwell/engine-deps.manifest lib/cmake/absl/abslConfig.cmake \
        "lib/${spm_lib}" include/sentencepiece_processor.h; do
        [ -e "${deps_dir}/${f}" ] || {
            echo "error: ENGINE_DEPS_DIR has no ${f}: install it with build-sentencepiece-abseil.sh" >&2
            exit 1
        }
    done
    deps+=(
        "-DCMAKE_PREFIX_PATH=${deps_dir}"
        "-Dabsl_DIR=${deps_dir}/lib/cmake/absl"
        "-DSENTENCEPIECE_LIB=${deps_lib}/${spm_lib}${spm_absl}"
        "-DSENTENCEPIECE_INCLUDE_DIR=${deps_dir}/include"
        -DCMAKE_DISABLE_FIND_PACKAGE_unofficial-sentencepiece=ON
        -DCMAKE_INSTALL_RPATH_USE_LINK_PATH=ON
    )
    sed -n 's/^source \([^ ]*\) \([^ ]*\) .*/using \1 \2 (pinned) from ENGINE_DEPS_DIR/p' \
        "${deps_dir}/share/inkwell/engine-deps.manifest"
elif [ "${macos}" = 1 ]; then
    echo "warning: ENGINE_DEPS_DIR is not set: SentencePiece and Abseil are whatever CMake finds" >&2
    echo "         (Homebrew). Fine for this Mac's tests; a release bundles the pinned builds." >&2
fi

mkdir -p "${prefix}"
prefix="$(native "$(cd "${prefix}" && pwd -P)")"
build="$(native "${build}")"
cmake --fresh -S "$(native "${src}")" -B "${build}" --preset "${preset}" \
    -DNEMO_SPEECH_BUILD_CLI=OFF \
    -DNEMO_SPEECH_BUILD_MIC_CAPTURE=OFF \
    -DGGML_NATIVE=OFF \
    ${platform[@]+"${platform[@]}"} \
    ${deps[@]+"${deps[@]}"} \
    -DCMAKE_INSTALL_PREFIX="${prefix}"
# With the pinned builds: the SentencePiece and Abseil CMake settled on are theirs, read back
# rather than assumed (a vcpkg config, or a static archive, would take their place silently).
if [ -n "${deps_lib}" ]; then
    cached() { sed -n "s/^$1:[A-Z]*=//p" "${build}/CMakeCache.txt"; }
    for var in SENTENCEPIECE_LIB SENTENCEPIECE_INCLUDE_DIR absl_DIR; do
        value="$(cached "${var}")"
        case "${value}" in
            "${deps_dir}"/*) ;;
            *)
                echo "error: CMake configured ${var}='${value}', not a path in ENGINE_DEPS_DIR" >&2
                exit 1
                ;;
        esac
    done
    for var in unofficial-sentencepiece_DIR SENTENCEPIECE_STATIC_LIB; do
        value="$(cached "${var}")"
        case "${value}" in
            "" | *-NOTFOUND) ;;
            *)
                echo "error: CMake configured ${var}='${value}': another SentencePiece than ENGINE_DEPS_DIR's" >&2
                exit 1
                ;;
        esac
    done
fi
cmake --build "${build}"
cmake --install "${build}"

# What CMake actually used, read back rather than assumed.
ggml_native="$(sed -n 's/^GGML_NATIVE:BOOL=//p' "${build}/CMakeCache.txt")"
if [ "${ggml_native}" != OFF ]; then
    echo "error: ${build}/CMakeCache.txt has GGML_NATIVE=${ggml_native:-unset}, not OFF" >&2
    exit 1
fi

lib="${prefix}/lib"

# --- macOS: a self-contained prefix --------------------------------------------------------------
if [ "${macos}" = 1 ]; then
    # NeMo's own libraries, as installed.
    installed=()
    while IFS= read -r f; do
        case "${f}" in
            "${lib}"/*.dylib) [ -f "${f}" ] && [ ! -L "${f}" ] && installed+=("${f}") ;;
        esac
    done <"${build}/install_manifest.txt"
    [ "${#installed[@]}" -gt 0 ] || { echo "error: the install put no library in ${lib}" >&2; exit 1; }
    make_self_contained "${lib}" "${deployment_target}" "${installed[@]}"
    # With the pinned builds: SentencePiece and Abseil were copied in, and every copy is theirs.
    if [ -n "${deps_lib}" ]; then
        check_pinned_origins "${deps_lib}"
    fi
fi

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum <"$1" | cut -d' ' -f1
    else
        shasum -a 256 <"$1" | cut -d' ' -f1
    fi
}
manifest="${prefix}/share/inkwell/nemo-speech.manifest"
mkdir -p "$(dirname "${manifest}")"
{
    echo "# NeMo-Speech.cpp as installed by build-nemo-speech.sh; ink-engines' build.rs checks it."
    echo "commit ${NEMO_COMMIT}"
    echo "ggml_native ${ggml_native}"
    if [ "${macos}" = 1 ]; then
        echo "# Copied into lib/ from outside the OS (each library's source, for its licence):"
        for o in ${origins[@]+"${origins[@]}"}; do
            echo "# bundled ${o}"
        done
    fi
    if [ "${windows}" = 1 ]; then
        echo "# Windows: the DLLs this build installed in bin/ and the C API's import library;"
        echo "# SentencePiece and Abseil are linked statically from ENGINE_DEPS_DIR."
        # What this build installed, not whatever the prefix already held.
        while IFS= read -r f; do
            f="${f%$'\r'}" # CMake writes the file with CRLF line ends on Windows.
            case "${f}" in
                "${prefix}"/bin/*.dll | "${prefix}"/lib/nemo_speech_asr_c.lib)
                    echo "sha256 $(sha256 "${f}") ${f#"${prefix}"/}"
                    ;;
            esac
        done <"${build}/install_manifest.txt"
    else
        (
            cd "${prefix}"
            for f in lib/*; do
                if [ -f "${f}" ] && [ ! -L "${f}" ]; then
                    case "${f}" in
                        *.dylib | *.so | *.so.*) echo "sha256 $(sha256 "${f}") ${f}" ;;
                    esac
                fi
            done
        )
    fi
} >"${manifest}"

echo
echo "Installed to ${prefix} (GGML_NATIVE=${ggml_native}). Build the adapter with:"
echo "  NEMO_SPEECH_DIR=${prefix} cargo build -p ink-engines --features engine-nemo"
