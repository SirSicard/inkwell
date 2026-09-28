#!/usr/bin/env bash
# Builds the two libraries NeMo-Speech.cpp loads from outside the OS, SentencePiece and Abseil
# (both Apache-2.0, by Google), from pinned release tarballs, so that a Mac release bundles the
# same versions every time instead of whatever a package manager serves that day.
#
#   build-sentencepiece-abseil.sh <prefix> <work dir>
#
# <prefix>    where to install: <prefix>/lib the libraries, <prefix>/include their headers,
#             <prefix>/lib/cmake/absl Abseil's CMake package, <prefix>/share/licenses their licence
#             files, and <prefix>/share/inkwell/engine-deps.manifest the versions, each tarball's
#             SHA-256 and each library's. It must not exist yet, or be empty: nothing is built over
#             an earlier install. Pass it to build-nemo-speech.sh as ENGINE_DEPS_DIR.
# <work dir>  the downloads (<work dir>/downloads, kept), and the sources and build trees
#             (<work dir>/src and <work dir>/build, replaced on every run).
#
# Each tarball is fetched by the URL below unless <work dir>/downloads already holds it, and either
# way is checked against the SHA-256 below before anything is extracted: a mismatch stops the
# script. (A download that fails the check is deleted; a file that was already there is left for
# you to look at.)
#
# Built to ship, as build-nemo-speech.sh builds NeMo-Speech.cpp:
# - arm64 only, for MACOSX_DEPLOYMENT_TARGET (else 26.0, mac/Package.swift's platform), with no
#   host-specific flag: nothing here passes -march or -mcpu, and neither project adds a native one
#   (Abseil's only CPU flag is its hardware-AES Randen kernel, chosen at run time on arm64).
# - Shared libraries, each installed as @rpath/<name> with `@loader_path` as its only rpath: the
#   layout the app's Contents/Frameworks has, so build-nemo-speech.sh copies them into its prefix
#   unchanged but for their signature, and mac/scripts/lib/bundle-check.sh accepts them.
# - SentencePiece against this Abseil, not the copy of Abseil its tarball carries: NeMo-Speech.cpp
#   includes sentencepiece_processor.h, which since 0.2.1 includes Abseil's headers and passes
#   absl::Status across the library boundary, so NeMo and SentencePiece must be compiled against
#   one Abseil. (SentencePiece's own default fetches Abseil with git at configure time; that path
#   is never taken here.) Its tarball's third_party/absl and third_party/abseil-cpp are deleted
#   before configuring, so no header of that other Abseil can be picked up; SentencePiece then
#   links its third_party/absl name to this Abseil's headers. Its other third-party code,
#   protobuf-lite and Darts-clone (BSD-3-Clause), is compiled into its library as upstream ships it.
# - TCMalloc off: SentencePiece would otherwise link whichever one the build machine has.
#
# The SentencePiece release asset is the Python source distribution, which carries the complete C++
# source under sentencepiece/: that directory is what gets built.
#
# Needs CMake 3.24 or later, Ninja and curl (on macOS `brew install cmake ninja`).
set -euo pipefail

ABSEIL_VERSION=20260817.0
ABSEIL_TARBALL=abseil-cpp-20260817.0.tar.gz
ABSEIL_URL=https://github.com/abseil/abseil-cpp/releases/download/20260817.0/abseil-cpp-20260817.0.tar.gz
ABSEIL_SHA256=f7e05179df39c45434cad433f5783840bb3788ef322976f9138bc6b72b3a107d
SENTENCEPIECE_VERSION=0.2.2
SENTENCEPIECE_TARBALL=sentencepiece-0.2.2.tar.gz
SENTENCEPIECE_URL=https://github.com/google/sentencepiece/releases/download/v0.2.2/sentencepiece-0.2.2.tar.gz
SENTENCEPIECE_SHA256=3d2b5e824b5622038dc7b490897efe05ebbbb9e7350fc142f3ecc8789ef9bdf6

fail() { echo "build-sentencepiece-abseil: $*" >&2; exit 1; }

if [ "$#" -ne 2 ]; then
    sed -n '6,15p' "$0" >&2
    exit 2
fi
prefix="$1"
work="$2"

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum <"$1" | cut -d' ' -f1
    else
        shasum -a 256 <"$1" | cut -d' ' -f1
    fi
}

# fetch <tarball> <url> <sha256>: the checked tarball in the downloads, or the script stops.
fetch() {
    local name="$1" url="$2" want="$3" file got
    file="${downloads}/${name}"
    if [ ! -e "${file}" ]; then
        echo "fetching ${url}"
        rm -f "${file}.part"
        curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
            --retry 3 --output "${file}.part" "${url}"
        got="$(sha256 "${file}.part")"
        if [ "${got}" != "${want}" ]; then
            rm -f "${file}.part"
            fail "${name} from ${url} has SHA-256 ${got}, not the pinned ${want}: refused (deleted)"
        fi
        mv "${file}.part" "${file}"
    fi
    got="$(sha256 "${file}")"
    [ "${got}" = "${want}" ] \
        || fail "${file} has SHA-256 ${got}, not the pinned ${want}: refused (delete it to fetch it again)"
    echo "${name}: sha256 ${got} (pinned)"
}

# Checked before anything is fetched or built.
if [ -e "${prefix}" ] && [ -n "$(ls -A "${prefix}")" ]; then
    fail "${prefix} is not empty: install into a new prefix"
fi
downloads="${work}/downloads"
mkdir -p "${downloads}"
fetch "${ABSEIL_TARBALL}" "${ABSEIL_URL}" "${ABSEIL_SHA256}"
fetch "${SENTENCEPIECE_TARBALL}" "${SENTENCEPIECE_URL}" "${SENTENCEPIECE_SHA256}"

[ "$(uname -s)" = Darwin ] || fail "builds the Mac app's libraries: run it on macOS"
deployment_target="${MACOSX_DEPLOYMENT_TARGET:-26.0}"

mkdir -p "${prefix}"
prefix="$(cd "${prefix}" && pwd -P)"
work="$(cd "${work}" && pwd -P)"
src="${work}/src"
build="${work}/build"
rm -rf "${src}" "${build}"
mkdir -p "${src}" "${build}"
tar -xzf "${downloads}/${ABSEIL_TARBALL}" -C "${src}"
tar -xzf "${downloads}/${SENTENCEPIECE_TARBALL}" -C "${src}"
absl_src="${src}/abseil-cpp-${ABSEIL_VERSION}"
spm_src="${src}/sentencepiece-${SENTENCEPIECE_VERSION}/sentencepiece"
[ -f "${absl_src}/CMakeLists.txt" ] || fail "${ABSEIL_TARBALL} has no abseil-cpp-${ABSEIL_VERSION}/CMakeLists.txt"
[ -f "${spm_src}/CMakeLists.txt" ] || fail "${SENTENCEPIECE_TARBALL} has no sentencepiece/CMakeLists.txt"
[ "$(cat "${spm_src}/VERSION.txt")" = "${SENTENCEPIECE_VERSION}" ] \
    || fail "${SENTENCEPIECE_TARBALL}'s VERSION.txt is not ${SENTENCEPIECE_VERSION}"
rm -rf "${spm_src}/third_party/absl" "${spm_src}/third_party/abseil-cpp"

# Both projects, the same way: Release, arm64 for the deployment target, installed by @rpath.
common=(
    -G Ninja
    -DCMAKE_BUILD_TYPE=Release
    -DCMAKE_OSX_ARCHITECTURES=arm64
    "-DCMAKE_OSX_DEPLOYMENT_TARGET=${deployment_target}"
    -DCMAKE_CXX_STANDARD=17
    "-DCMAKE_INSTALL_PREFIX=${prefix}"
    -DCMAKE_INSTALL_LIBDIR=lib
    -DCMAKE_INSTALL_NAME_DIR=@rpath
    -DCMAKE_INSTALL_RPATH=@loader_path
)

cmake --fresh -S "${absl_src}" -B "${build}/abseil" "${common[@]}" \
    -DBUILD_SHARED_LIBS=ON \
    -DABSL_PROPAGATE_CXX_STD=ON \
    -DABSL_ENABLE_INSTALL=ON \
    -DABSL_BUILD_TESTING=OFF \
    -DBUILD_TESTING=OFF
cmake --build "${build}/abseil"
cmake --install "${build}/abseil"

# The Abseil just installed, and nothing else: its package by path, and the search for others off.
cmake --fresh -S "${spm_src}" -B "${build}/sentencepiece" "${common[@]}" \
    -DSPM_ABSL_PROVIDER=package \
    "-Dabsl_DIR=${prefix}/lib/cmake/absl" \
    -DCMAKE_FIND_PACKAGE_PREFER_CONFIG=ON \
    -DSPM_PROTOBUF_PROVIDER=internal \
    -DSPM_ENABLE_SHARED=ON \
    -DSPM_ENABLE_TCMALLOC=OFF \
    -DSPM_ENABLE_NFKC_COMPILE=OFF \
    -DSPM_BUILD_TEST=OFF
# Read back: the Abseil it configured against is this prefix's.
used="$(sed -n 's/^absl_DIR:[A-Z]*=//p' "${build}/sentencepiece/CMakeCache.txt")"
[ "${used}" = "${prefix}/lib/cmake/absl" ] || fail "SentencePiece was configured against the Abseil at '${used}'"
cmake --build "${build}/sentencepiece"
cmake --install "${build}/sentencepiece"

# The licences that ship with the libraries (the app's About screen shows them).
licences="${prefix}/share/licenses"
mkdir -p "${licences}/abseil" "${licences}/sentencepiece/protobuf-lite" "${licences}/sentencepiece/darts_clone"
cp "${absl_src}/LICENSE" "${licences}/abseil/LICENSE"
cp "${spm_src}/LICENSE" "${licences}/sentencepiece/LICENSE"
cp "${spm_src}/third_party/protobuf-lite/LICENSE" "${licences}/sentencepiece/protobuf-lite/LICENSE"
cp "${spm_src}/third_party/darts_clone/LICENSE" "${licences}/sentencepiece/darts_clone/LICENSE"

# --- the layout Contents/Frameworks needs, checked ------------------------------------------------
# Every library loads the others as @rpath/<name> and finds them by `@loader_path` alone (CMake
# leaves SentencePiece's own absolute rpath in: it sets one itself). Anything else it loads must be
# the OS's. A library whose load commands change is signed again, ad hoc, as build-nemo-speech.sh
# does (the app signs its own copies with its identity).
lib="${prefix}/lib"
libs=()
for f in "${lib}"/*.dylib; do
    [ -f "${f}" ] && [ ! -L "${f}" ] && libs+=("${f}")
done
[ "${#libs[@]}" -gt 0 ] || fail "the install put no library in ${lib}"
for f in "${libs[@]}"; do
    args=()
    has_loader=0
    while IFS= read -r rp; do
        if [ "${rp}" = @loader_path ] && [ "${has_loader}" = 0 ]; then
            has_loader=1
        else
            args+=(-delete_rpath "${rp}")
        fi
    done < <(otool -l "${f}" | awk '$1 == "cmd" && $2 == "LC_RPATH" { getline; getline; print $2 }')
    [ "${has_loader}" = 1 ] || args+=(-add_rpath @loader_path)
    if [ "${#args[@]}" -gt 0 ]; then
        install_name_tool "${args[@]}" "${f}" 2>&1 | { grep -v 'invalidate the code signature' || true; } >&2
        codesign --force --sign - "${f}" 2>&1 | { grep -v 'replacing existing signature' || true; } >&2
    fi
done
for f in "${libs[@]}"; do
    name="$(basename "${f}")"
    id="$(otool -D "${f}" | sed -n '2p')"
    case "${id}" in
        @rpath/*) [ -e "${lib}/${id#@rpath/}" ] || fail "${name}'s install name ${id} names no library beside it" ;;
        *) fail "${name}'s install name is ${id}, not @rpath/<name>" ;;
    esac
    while IFS= read -r dep; do
        case "${dep}" in
            /usr/lib/* | /System/Library/*) ;;
            @rpath/*) [ -e "${lib}/${dep#@rpath/}" ] || fail "${name} loads ${dep}, which is not beside it" ;;
            *) fail "${name} loads ${dep}: not the OS's, and not beside it by @rpath" ;;
        esac
    done < <(otool -L "${f}" | sed -nE '2,$ s/^[[:space:]]+([^ ]+) \(.*/\1/p')
    rps="$(otool -l "${f}" | awk '$1 == "cmd" && $2 == "LC_RPATH" { getline; getline; print $2 }' | tr '\n' ' ')"
    [ "${rps}" = "@loader_path " ] || fail "${name} has the rpaths [${rps}], not @loader_path alone"
    archs="$(lipo -archs "${f}")"
    [ "${archs}" = arm64 ] || fail "${name} is built for [${archs}], not arm64 alone"
    minos="$(otool -l "${f}" | awk '$2 == "LC_BUILD_VERSION" { want = 1; next } want && $1 == "minos" { print $2; exit }')"
    [ "${minos%.0}" = "${deployment_target%.0}" ] \
        || fail "${name} is built for macOS ${minos:-?}, not ${deployment_target}"
done

# --- the manifest ---------------------------------------------------------------------------------
# build-nemo-speech.sh's manifest records where each library it bundles was copied from, and
# mac/scripts/build-manifest.sh reads this one to name its version and check its hash.
manifest="${prefix}/share/inkwell/engine-deps.manifest"
mkdir -p "$(dirname "${manifest}")"
{
    echo "# SentencePiece and Abseil as built by build-sentencepiece-abseil.sh, from pinned tarballs."
    echo "source abseil ${ABSEIL_VERSION} ${ABSEIL_SHA256} ${ABSEIL_TARBALL}"
    echo "source sentencepiece ${SENTENCEPIECE_VERSION} ${SENTENCEPIECE_SHA256} ${SENTENCEPIECE_TARBALL}"
    echo "deployment_target ${deployment_target}"
    (
        cd "${prefix}"
        for f in lib/*.dylib; do
            [ -f "${f}" ] && [ ! -L "${f}" ] || continue
            case "${f}" in
                lib/libabsl_*) component=abseil ;;
                lib/libsentencepiece*) component=sentencepiece ;;
                *) echo "build-sentencepiece-abseil: ${f} belongs to neither project" >&2; exit 1 ;;
            esac
            echo "lib ${component} $(sha256 "${f}") ${f}"
        done
    )
} >"${manifest}"

echo
echo "Installed SentencePiece ${SENTENCEPIECE_VERSION} and Abseil ${ABSEIL_VERSION} to ${prefix}"
echo "(arm64, macOS ${deployment_target}). Build NeMo-Speech.cpp against them with:"
echo "  ENGINE_DEPS_DIR=${prefix} core/crates/ink-engines/native/build-nemo-speech.sh <source> <prefix>"
