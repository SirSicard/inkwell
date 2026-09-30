# The Windows architecture and compiler the native build scripts build with (sourced by
# ../build-sentencepiece-abseil.sh and ../build-nemo-speech.sh; tests:
# mac/scripts/tests/test-windows-toolchain.sh, which the Mac workflows' script tests run).
#
#   windows_toolchain
#
# Reads the architecture from the Visual Studio developer environment the scripts run in
# (VSCMD_ARG_TGT_ARCH, which vcvarsall.bat and VsDevCmd.bat set) and sets:
# - win_arch: x64 or arm64; any other target is refused.
# - win_cc: the C and C++ compiler for every library in the chain. On x64, cl. On arm64, clang-cl:
#   ggml's CPU backend stops MSVC on ARM ("MSVC is not supported for ARM, use clang"), and
#   SentencePiece and Abseil are compiled by the same compiler as NeMo-Speech.cpp, which is
#   compiled against their headers and links them statically, so Abseil is configured the same way
#   on both sides.
# - win_cc_name: that compiler as the manifest names it: msvc or clang-cl.
# The compiler must be on PATH. On arm64, clang-cl must target aarch64 by default (an ARM64 LLVM on
# an ARM64 machine): its target is read from `clang-cl --version`, so an x64 LLVM cannot compile
# x64 objects into an ARM64 build. A problem is printed to stderr and returns 1.
windows_toolchain() {
    local target
    win_arch="${VSCMD_ARG_TGT_ARCH:-}"
    case "${win_arch}" in
        x64)
            win_cc=cl
            win_cc_name=msvc
            ;;
        arm64)
            win_cc=clang-cl
            win_cc_name=clang-cl
            ;;
        "")
            echo "error: VSCMD_ARG_TGT_ARCH is not set: run from a Visual Studio developer environment" >&2
            echo "       (vcvarsall.bat x64, or vcvarsall.bat arm64 on an ARM64 machine)" >&2
            return 1
            ;;
        *)
            echo "error: the developer environment targets '${win_arch}': the Windows app is built for x64 and arm64" >&2
            return 1
            ;;
    esac
    command -v "${win_cc}" >/dev/null 2>&1 || {
        echo "error: no ${win_cc} on PATH: ${win_arch} builds with ${win_cc}" >&2
        return 1
    }
    if [ "${win_arch}" = arm64 ]; then
        # Windows programs may end their lines with CR.
        target="$(clang-cl --version 2>/dev/null | tr -d '\r' | sed -n 's/^Target: //p')"
        case "${target}" in
            aarch64-*-windows-msvc) ;;
            *)
                echo "error: clang-cl targets '${target}', not aarch64-pc-windows-msvc: build ARM64 with an" >&2
                echo "       ARM64 LLVM on an ARM64 machine" >&2
                return 1
                ;;
        esac
    fi
}
