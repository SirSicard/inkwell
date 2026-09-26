#!/usr/bin/env bash
# Builds and installs the NeMo-Speech.cpp diarization library that `engine-nemo` links against.
#
#   build-nemo-speech.sh <source> <prefix> [build-dir]
#
# <source>  a NeMo-Speech.cpp checkout (Apache-2.0, github.com/NVIDIA/NeMo-Speech.cpp) at the
#           pinned commit below, with its ggml submodule initialised
#           (`git submodule update --init ggml`). Nothing is downloaded here.
# <prefix>  where to install: <prefix>/lib holds libnemo_speech_asr_c and NeMo's own ggml
#           libraries, <prefix>/include/nemo_speech the C headers, and
#           <prefix>/share/inkwell/nemo-speech.manifest the commit and each library's SHA-256.
#           Point NEMO_SPEECH_DIR at it when building ink-engines with `--features engine-nemo`.
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
# Needs CMake, Ninja, and SentencePiece and abseil (Apache-2.0; on macOS
# `brew install cmake ninja sentencepiece abseil`). The configuration is the upstream
# `metal-diar` preset (Metal, standalone diarization, unpatched-ggml code paths) with the CLI off.
set -euo pipefail

NEMO_COMMIT=97a15afa5caa9bce5baaa86c1184103877af4101
GGML_COMMIT=c03b4e2bcece5134827881af90242086daf75be5

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    sed -n '2,12p' "$0" >&2
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

cmake -S "${src}" -B "${build}" --preset metal-diar \
    -DNEMO_SPEECH_BUILD_CLI=OFF \
    -DNEMO_SPEECH_BUILD_MIC_CAPTURE=OFF \
    -DCMAKE_INSTALL_PREFIX="${prefix}"
cmake --build "${build}"
cmake --install "${build}"

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
} >"${manifest}"

echo
echo "Installed to ${prefix}. Build the adapter with:"
echo "  NEMO_SPEECH_DIR=${prefix} cargo build -p ink-engines --features engine-nemo"
