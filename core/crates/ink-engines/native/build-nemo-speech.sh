#!/usr/bin/env bash
# Builds and installs the NeMo-Speech.cpp diarization library that `engine-nemo` links against.
#
#   build-nemo-speech.sh <source> <prefix> [build-dir]
#
# <source>  a NeMo-Speech.cpp checkout (Apache-2.0, github.com/NVIDIA/NeMo-Speech.cpp) at the
#           pinned commit below, with its ggml submodule initialised
#           (`git submodule update --init ggml`). Nothing is downloaded here.
# <prefix>  where to install: <prefix>/lib holds libnemo_speech_asr_c, NeMo's own ggml libraries
#           and every library they load from outside the OS, <prefix>/include/nemo_speech the C
#           headers, and <prefix>/share/inkwell/nemo-speech.manifest the commit, the build's
#           GGML_NATIVE and each library's SHA-256. Point NEMO_SPEECH_DIR at it when building
#           ink-engines with `--features engine-nemo`.
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
#   (SentencePiece and Abseil, from Homebrew) is copied into <prefix>/lib, and every library there
#   is loaded by @rpath, with `@loader_path` as its only rpath. A copy was signed by its builder;
#   after its load commands change it is signed again, ad hoc (the app re-signs it with its own
#   identity). Nothing in the prefix names Homebrew or any other absolute path outside /usr/lib
#   and /System/Library, which is checked before the manifest is written.
# - CMake is configured afresh (`--fresh`), so no cached value from an earlier configuration of
#   the same build directory (a GGML_NATIVE=ON, a CPU feature probe) carries over.
#
# Needs CMake 3.24 or later, Ninja, and SentencePiece and abseil (Apache-2.0; on macOS
# `brew install cmake ninja sentencepiece abseil`). The configuration is the upstream
# `metal-diar` preset (Metal, standalone diarization, unpatched-ggml code paths) with the CLI off.
set -euo pipefail

NEMO_COMMIT=97a15afa5caa9bce5baaa86c1184103877af4101
GGML_COMMIT=c03b4e2bcece5134827881af90242086daf75be5

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    sed -n '2,13p' "$0" >&2
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
if [ "$(uname -s)" = Darwin ]; then
    macos=1
    deployment_target="${MACOSX_DEPLOYMENT_TARGET:-26.0}"
    platform+=("-DCMAKE_OSX_DEPLOYMENT_TARGET=${deployment_target}")
fi

cmake --fresh -S "${src}" -B "${build}" --preset metal-diar \
    -DNEMO_SPEECH_BUILD_CLI=OFF \
    -DNEMO_SPEECH_BUILD_MIC_CAPTURE=OFF \
    -DGGML_NATIVE=OFF \
    ${platform[@]+"${platform[@]}"} \
    -DCMAKE_INSTALL_PREFIX="${prefix}"
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
is_system() {
    case "$1" in
        /usr/lib/* | /System/Library/*) return 0 ;;
        *) return 1 ;;
    esac
}
# The libraries a Mach-O loads, without its own install name.
loads_of() {
    local id
    id="$(otool -D "$1" | sed -n '2p')"
    otool -L "$1" | sed -nE '2,$ s/^[[:space:]]+([^ ]+) \(.*/\1/p' | { grep -vxF -- "${id:-/}" || true; }
}
rpaths_of() {
    otool -l "$1" | awk '$1 == "cmd" && $2 == "LC_RPATH" { getline; getline; print $2 }'
}
# The oldest macOS a Mach-O runs on.
minos_of() {
    otool -l "$1" | awk '
        $2 == "LC_BUILD_VERSION" || $2 == "LC_VERSION_MIN_MACOSX" { want = 1; next }
        want && ($1 == "minos" || $1 == "version") { print $2; exit }'
}
# Whether version $1 is newer than version $2 (dotted numbers).
newer_than() {
    [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -t. -k1,1n -k2,2n -k3,3n | tail -1)" = "$1" ]
}
# Where the library `dep`, as `from` (a file at its original place) names it, is found.
resolve() {
    local from="$1" dep="$2" dir rp candidate
    dir="$(dirname "${from}")"
    case "${dep}" in
        /*) [ -e "${dep}" ] && { echo "${dep}"; return 0; } ;;
        @loader_path/*)
            candidate="${dir}/${dep#@loader_path/}"
            [ -e "${candidate}" ] && { echo "${candidate}"; return 0; }
            ;;
        @rpath/*)
            while IFS= read -r rp; do
                case "${rp}" in
                    @loader_path*) candidate="${dir}${rp#@loader_path}/${dep#@rpath/}" ;;
                    /*) candidate="${rp}/${dep#@rpath/}" ;;
                    *) continue ;;
                esac
                [ -e "${candidate}" ] && { echo "${candidate}"; return 0; }
            done < <(rpaths_of "${from}")
            ;;
    esac
    return 1
}

if [ "${macos}" = 1 ]; then
    # NeMo's own libraries, as installed.
    installed=()
    while IFS= read -r f; do
        case "${f}" in
            "${lib}"/*.dylib) [ -f "${f}" ] && [ ! -L "${f}" ] && installed+=("${f}") ;;
        esac
    done <"${build}/install_manifest.txt"
    [ "${#installed[@]}" -gt 0 ] || { echo "error: the install put no library in ${lib}" >&2; exit 1; }

    # Everything they load from outside the OS, and what that loads in turn, copied beside them
    # under the name it is loaded by. `queue` holds files at their original places, so a copy's
    # own @rpath and @loader_path references resolve as its builder meant them to.
    copied=()
    origins=()
    queue=("${installed[@]}")
    while [ "${#queue[@]}" -gt 0 ]; do
        from="${queue[0]}"
        queue=("${queue[@]:1}")
        own=0
        case "${from}" in "${lib}"/*) own=1 ;; esac
        while IFS= read -r dep; do
            is_system "${dep}" && continue
            name="$(basename "${dep}")"
            # NeMo's own libraries load each other by @rpath: those are all in the prefix already.
            if [ "${own}" = 1 ] && [ "${dep#@rpath/}" != "${dep}" ] && [ -e "${lib}/${name}" ]; then
                continue
            fi
            case " ${copied[*]-} " in *" ${name} "*) continue ;; esac
            found="$(resolve "${from}" "${dep}")" || {
                echo "error: ${from} loads ${dep}, which is not found" >&2
                exit 1
            }
            real="$(realpath "${found}")"
            rm -f "${lib}/${name}"
            cp "${real}" "${lib}/${name}"
            chmod u+w "${lib}/${name}"
            copied+=("${name}")
            origins+=("${name} ${real}")
            queue+=("${real}")
        done < <(loads_of "${from}")
    done

    # Every library by @rpath, with only `@loader_path` to find them: the prefix's own directory,
    # which in the app is Contents/Frameworks.
    for f in "${installed[@]}" ${copied[@]+"${copied[@]/#/${lib}/}"}; do
        args=()
        name="$(basename "${f}")"
        case " ${copied[*]-} " in *" ${name} "*) args+=(-id "@rpath/${name}") ;; esac
        while IFS= read -r dep; do
            case "${dep}" in
                /*) is_system "${dep}" || args+=(-change "${dep}" "@rpath/$(basename "${dep}")") ;;
            esac
        done < <(loads_of "${f}")
        has_loader=0
        while IFS= read -r rp; do
            if [ "${rp}" = @loader_path ] && [ "${has_loader}" = 0 ]; then
                has_loader=1
            else
                args+=(-delete_rpath "${rp}")
            fi
        done < <(rpaths_of "${f}")
        [ "${has_loader}" = 1 ] || args+=(-add_rpath @loader_path)
        if [ "${#args[@]}" -gt 0 ]; then
            # Its only warning is that the signature no longer matches, which the next line fixes.
            install_name_tool "${args[@]}" "${f}" 2>&1 | { grep -v 'invalidate the code signature' || true; } >&2
            codesign --force --sign - "${f}" 2>&1 | { grep -v 'replacing existing signature' || true; } >&2
        fi
    done

    # The check: every library in the prefix loads only the OS and its neighbours, finds them by
    # `@loader_path` alone, and was built for the deployment target or older.
    for f in "${lib}"/*.dylib; do
        [ -f "${f}" ] && [ ! -L "${f}" ] || continue
        name="$(basename "${f}")"
        while IFS= read -r dep; do
            is_system "${dep}" && continue
            case "${dep}" in
                @rpath/*) [ -e "${lib}/${dep#@rpath/}" ] && continue ;;
            esac
            echo "error: ${name} loads ${dep}: not a system library, and not beside it by @rpath" >&2
            exit 1
        done < <(otool -L "${f}" | sed -nE '2,$ s/^[[:space:]]+([^ ]+) \(.*/\1/p')
        rps="$(rpaths_of "${f}" | tr '\n' ' ')"
        if [ "${rps}" != "@loader_path " ]; then
            echo "error: ${name} has the rpaths [${rps}], not @loader_path alone" >&2
            exit 1
        fi
        minos="$(minos_of "${f}")"
        if [ -z "${minos}" ] || newer_than "${minos}" "${deployment_target}"; then
            # A warning here, not a failure: a Homebrew built for a newer macOS than the target
            # (bottles are built per macOS release) leaves nothing older to copy on this Mac. The
            # prefix still serves this Mac's tests; mac/scripts/build-mac.sh refuses to ship it.
            echo "warning: ${name} is built for macOS ${minos:-?}, newer than ${deployment_target}:" >&2
            echo "         an app bundling it will not start on older macOS" >&2
        fi
    done
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
echo "Installed to ${prefix} (GGML_NATIVE=${ggml_native}). Build the adapter with:"
echo "  NEMO_SPEECH_DIR=${prefix} cargo build -p ink-engines --features engine-nemo"
