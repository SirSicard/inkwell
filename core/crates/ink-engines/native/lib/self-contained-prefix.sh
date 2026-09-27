# A self-contained macOS install prefix (sourced by ../build-nemo-speech.sh; tests:
# mac/scripts/tests/test-self-contained-prefix.sh, which the Mac workflows' script tests run).
#
#   make_self_contained <lib> <deployment target> <installed library...>
#
# <lib> is the prefix's library directory, and the installed libraries are the dylibs a build put
# there. Afterwards:
# - everything they load from outside the OS (/usr/lib, /System/Library), and what that loads in
#   turn, is copied into <lib> under the name it is loaded by (the walk ends at a name already
#   copied, so a cycle ends too; a library that cannot be found stops it);
# - every library there loads the others by @rpath, with `@loader_path` as its only rpath, and a
#   copy's install name is @rpath/<name>; a library whose load commands changed is signed again,
#   ad hoc (its builder's signature no longer matches it; the app re-signs it with its own
#   identity);
# - which is checked: every library in <lib> loads only the OS and its neighbours, finds them by
#   `@loader_path` alone, and was built for the deployment target or older (only a warning).
# The copies' names are left in the array `copied`, and each one's source in `origins`
# ("<name> <real path>"): the build script's manifest names them. A problem is printed to stderr
# and returns 1; call it as a plain command under `set -e`, so any other failing command stops it
# too.
#
# Kept apart from the build script so the Mach-O walking can be tested on tiny libraries without
# building NeMo-Speech.cpp.

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

make_self_contained() {
    local lib="$1" deployment_target="$2"
    shift 2
    local installed=("$@")
    local queue from own dep name found real f args has_loader rp rps minos
    copied=()
    origins=()

    # Everything they load from outside the OS, and what that loads in turn, copied beside them
    # under the name it is loaded by. `queue` holds files at their original places, so a copy's
    # own @rpath and @loader_path references resolve as its builder meant them to.
    queue=("${installed[@]}")
    while [ "${#queue[@]}" -gt 0 ]; do
        from="${queue[0]}"
        queue=("${queue[@]:1}")
        own=0
        case "${from}" in "${lib}"/*) own=1 ;; esac
        while IFS= read -r dep; do
            is_system "${dep}" && continue
            name="$(basename "${dep}")"
            # The prefix's own libraries load each other by @rpath: those are all in it already.
            if [ "${own}" = 1 ] && [ "${dep#@rpath/}" != "${dep}" ] && [ -e "${lib}/${name}" ]; then
                continue
            fi
            case " ${copied[*]-} " in *" ${name} "*) continue ;; esac
            found="$(resolve "${from}" "${dep}")" || {
                echo "error: ${from} loads ${dep}, which is not found" >&2
                return 1
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
            return 1
        done < <(otool -L "${f}" | sed -nE '2,$ s/^[[:space:]]+([^ ]+) \(.*/\1/p')
        rps="$(rpaths_of "${f}" | tr '\n' ' ')"
        if [ "${rps}" != "@loader_path " ]; then
            echo "error: ${name} has the rpaths [${rps}], not @loader_path alone" >&2
            return 1
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
}
