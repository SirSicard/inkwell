# Extracting a source tarball without letting it write outside its directory (sourced by
# ../build-sentencepiece-abseil.sh; tests: mac/scripts/tests/test-safe-extract.sh).
#
#   safe_extract <tarball> <directory>
#
# The tarball is listed first, and refused (nothing extracted) if any entry:
# - has an absolute name, or a `..` component in its name;
# - is a symbolic link whose target is absolute, or leads out of <directory> from where the link
#   sits;
# - is a hard link whose target is absolute or has a `..` component (a hard link's target is
#   another entry, named from the top of the archive);
# - has a name or link target with a control character (a newline, say) or a backslash, or a name
#   containing " -> " or " link to ": the listing is read line by line and split on those.
# bsdtar has no NUL-separated listing (its --null is for -T's input); it prints each entry on one
# line, with control characters and backslashes escaped as \n, \033, \\ and so on. So a backslash
# anywhere in the listing means one of those, and is refused: nothing then depends on how an odd
# name would have been printed. The verbose listing is paired with the plain one entry by entry,
# so a link's name is known before its line is split.
# Then it is extracted into <directory> with --no-same-owner. The checksum the build script checks
# first already pins what the tarball holds; this keeps a tarball that ever changed shape (or a
# pin updated in a hurry) from writing anywhere but its own tree. A problem is printed to stderr
# and returns 1.

# Whether a relative path, taken from the directory <depth> levels below the top, leaves the top.
# $1: that depth; $2: the path.
safe_extract_escapes() {
    local depth="$1" path="$2" part rest
    rest="${path}/"
    while [ -n "${rest}" ]; do
        part="${rest%%/*}"
        rest="${rest#*/}"
        case "${part}" in
            '' | .) ;;
            ..)
                depth=$((depth - 1))
                [ "${depth}" -ge 0 ] || return 0
                ;;
            *) depth=$((depth + 1)) ;;
        esac
    done
    return 1
}

# The number of directories above a name inside the archive ("a/b/c" -> 2).
safe_extract_depth() {
    local name="$1" rest part depth=0
    rest="${name%/}"
    case "${rest}" in */*) rest="${rest%/*}/" ;; *) rest="" ;; esac
    while [ -n "${rest}" ]; do
        part="${rest%%/*}"
        rest="${rest#*/}"
        case "${part}" in '' | .) ;; *) depth=$((depth + 1)) ;; esac
    done
    echo "${depth}"
}

safe_extract() {
    local tarball="$1" dest="$2" names listing line name target kind i
    local name_list=() line_list=()
    # Read whole first: a loop that stops early must not leave tar writing into a closed pipe.
    names="$(tar -tzf "${tarball}")" || { echo "error: cannot list ${tarball}" >&2; return 1; }
    listing="$(tar -tvzf "${tarball}")" || { echo "error: cannot list ${tarball}" >&2; return 1; }
    [ -n "${names}" ] || { echo "error: ${tarball} is empty" >&2; return 1; }
    case "${names}${listing}" in
        *\\*)
            echo "error: ${tarball} has a name or link target with a control character or a backslash: refused" >&2
            return 1
            ;;
    esac
    while IFS= read -r name; do
        case "${name}" in
            /*) echo "error: ${tarball} holds ${name}, an absolute path: refused" >&2; return 1 ;;
            *" -> "* | *" link to "*)
                echo "error: ${tarball} holds ${name}, a name its listing cannot be split around: refused" >&2
                return 1
                ;;
        esac
        case "/${name}/" in
            */../*) echo "error: ${tarball} holds ${name}, a path with '..': refused" >&2; return 1 ;;
        esac
        name_list+=("${name}")
    done <<<"${names}"
    while IFS= read -r line; do line_list+=("${line}"); done <<<"${listing}"
    [ "${#line_list[@]}" = "${#name_list[@]}" ] \
        || { echo "error: ${tarball}: its two listings disagree on the number of entries: refused" >&2; return 1; }
    # Links: each verbose line against its entry's name. The line ends with the name, then
    # " -> <target>" (symbolic) or " link to <target>" (hard); no name contains either.
    i=0
    while [ "${i}" -lt "${#name_list[@]}" ]; do
        name="${name_list[${i}]}"
        line="${line_list[${i}]}"
        i=$((i + 1))
        kind="${line:0:1}"
        [ "${kind}" = l ] || [ "${kind}" = h ] || continue
        if [ "${kind}" = l ]; then
            case "${line}" in
                *" ${name} -> "*) target="${line#*" ${name} -> "}" ;;
                *) echo "error: ${tarball}: cannot read the link ${name}'s target: refused" >&2; return 1 ;;
            esac
            case "${target}" in
                /*) echo "error: ${tarball}: the link ${name} points at ${target}, an absolute path: refused" >&2; return 1 ;;
            esac
            if safe_extract_escapes "$(safe_extract_depth "${name}")" "${target}"; then
                echo "error: ${tarball}: the link ${name} points at ${target}, outside the tree: refused" >&2
                return 1
            fi
        else
            case "${line}" in
                *" ${name} link to "*) target="${line#*" ${name} link to "}" ;;
                *) echo "error: ${tarball}: cannot read the hard link ${name}'s target: refused" >&2; return 1 ;;
            esac
            case "/${target}/" in
                //* | */../*)
                    echo "error: ${tarball}: the hard link ${name} points at ${target}, outside the tree: refused" >&2
                    return 1
                    ;;
            esac
        fi
    done
    mkdir -p "${dest}"
    tar -xzf "${tarball}" --no-same-owner -C "${dest}"
}
