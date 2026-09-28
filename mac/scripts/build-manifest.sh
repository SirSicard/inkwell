#!/usr/bin/env bash
# The build manifest a Mac release keeps beside its dmg: which third-party libraries the app
# bundles, the pinned sources they were built from, and the build tools' versions. The build log
# shows the same, but a log expires; a release asset lasts as long as the release, so a shipped dmg
# can always be traced to the SentencePiece and Abseil inside it. The release workflow attaches it
# to the draft release, and keeps it as the run's artifact (the dry run's only copy).
#
#   mac/scripts/build-manifest.sh --version X.Y.Z --commit <SHA-1> --dmg <dmg> \
#     --nemo <NeMo prefix>/share/inkwell/nemo-speech.manifest \
#     --deps <SentencePiece/Abseil prefix>/share/inkwell/engine-deps.manifest \
#     --homebrew <what `brew list --versions cmake ninja` printed> --out <file>
#
# The file is public, so it names no path on the build machine. build-nemo-speech.sh records
# where it copied each library from; every one must be a library build-sentencepiece-abseil.sh
# installed in the --deps prefix, listed in that prefix's manifest with the SHA-256 it still has,
# and that manifest's sources must be the tarballs the script pins (their versions and SHA-256s,
# read from the script). A library from anywhere else (Homebrew, say) is refused, not written: its
# version would be whatever that source served. Nothing is written unless all of it checks out.
set -euo pipefail

fail() { echo "build-manifest: $*" >&2; exit 1; }
usage="usage: build-manifest.sh --version X.Y.Z --commit <SHA-1> --dmg <dmg> --nemo <manifest> --deps <manifest> --homebrew <file> --out <file>"
root="$(cd "$(dirname "$0")/../.." && pwd)"
deps_script="$root/core/crates/ink-engines/native/build-sentencepiece-abseil.sh"

version="" commit="" dmg="" nemo="" deps="" homebrew="" out=""
while [ $# -gt 0 ]; do
  case "$1" in
    --version | --commit | --dmg | --nemo | --deps | --homebrew | --out)
      [ $# -ge 2 ] || fail "$1 needs a value"
      case "$1" in
        --version) version="$2" ;;
        --commit) commit="$2" ;;
        --dmg) dmg="$2" ;;
        --nemo) nemo="$2" ;;
        --deps) deps="$2" ;;
        --homebrew) homebrew="$2" ;;
        --out) out="$2" ;;
      esac
      shift
      ;;
    *) fail "unknown argument: $1 ($usage)" ;;
  esac
  shift
done
[ -n "$out" ] || fail "$usage"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "--version is X.Y.Z, not '$version'"
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || fail "--commit is a full commit hash (40 hex digits), not '$commit'"
[ -f "$dmg" ] || fail "no dmg at $dmg"
[ -f "$nemo" ] || fail "no NeMo-Speech.cpp manifest at $nemo"
[ -f "$deps" ] || fail "no SentencePiece/Abseil manifest at $deps"
[ -f "$homebrew" ] || fail "no Homebrew version list at $homebrew"

sha256() { shasum -a 256 <"$1" | cut -d' ' -f1; }

# The pins, as the build script states them.
pin() { sed -n "s/^$1=//p" "$deps_script"; }
pins=(
  "abseil $(pin ABSEIL_VERSION) $(pin ABSEIL_SHA256) $(pin ABSEIL_TARBALL)"
  "sentencepiece $(pin SENTENCEPIECE_VERSION) $(pin SENTENCEPIECE_SHA256) $(pin SENTENCEPIECE_TARBALL)"
)
for p in "${pins[@]}"; do
  [[ "$p" =~ ^[a-z]+\ [0-9.]+\ [0-9a-f]{64}\ [A-Za-z0-9._-]+\.tar\.gz$ ]] || fail "cannot read the pins in $deps_script"
done

# The pinned prefix's manifest, as build-sentencepiece-abseil.sh writes it: its sources must be the
# pins, and each library is known by its file name, with its project and SHA-256.
# The manifest sits in <prefix>/share/inkwell/, where build-sentencepiece-abseil.sh writes it; the
# libraries are checked against that prefix, so a manifest anywhere else is refused.
deps_dir="$(cd "$(dirname "$deps")" 2>/dev/null && pwd -P)" || fail "cannot resolve the directory of $deps"
case "$deps_dir" in
  */share/inkwell) deps_prefix="${deps_dir%/share/inkwell}" ;;
  *) fail "$deps is not in <prefix>/share/inkwell/, where build-sentencepiece-abseil.sh writes it" ;;
esac
sources=()
deps_libs=()
while IFS= read -r line; do
  read -r -a fields <<<"$line"
  case "${fields[0]-}" in
    '' | '#'*) continue ;;
    source)
      [ "${#fields[@]}" = 5 ] || fail "$deps: unreadable line '$line'"
      sources+=("${fields[1]} ${fields[2]} ${fields[3]} ${fields[4]}")
      ;;
    deployment_target) [ "${#fields[@]}" = 2 ] || fail "$deps: unreadable line '$line'" ;;
    lib)
      [ "${#fields[@]}" = 4 ] && [[ "${fields[2]}" =~ ^[0-9a-f]{64}$ ]] && [[ "${fields[3]}" =~ ^lib/[^/]+$ ]] \
        || fail "$deps: unreadable line '$line'"
      deps_libs+=("${fields[3]#lib/} ${fields[1]} ${fields[2]}")
      ;;
    *) fail "$deps: unreadable line '$line'" ;;
  esac
done <"$deps"
[ "${#sources[@]}" = "${#pins[@]}" ] || fail "$deps records ${#sources[@]} source(s), not the ${#pins[@]} pinned ones"
for p in "${pins[@]}"; do
  case " $(printf '%s|' "${sources[@]}") " in
    *"$p|"*) ;;
    *) fail "$deps was not built from the pinned ${p%% *} (${p#* })" ;;
  esac
done
[ "${#deps_libs[@]}" -gt 0 ] || fail "$deps lists no library"
# The version a project was built at, from its pin.
version_of() {
  local p
  for p in "${pins[@]}"; do
    [ "${p%% *}" = "$1" ] && { p="${p#* }"; echo "${p%% *}"; return 0; }
  done
  return 1
}

# The build tools' list (Homebrew's): `<formula> <version>...`, one formula a line.
brew_lines=()
while IFS= read -r line; do
  [ -n "$line" ] || continue
  [[ "$line" =~ ^[a-z0-9@+._-]+( [A-Za-z0-9+._-]+)+$ ]] || fail "$homebrew: '$line' is not a line of brew list --versions"
  brew_lines+=("$line")
done <"$homebrew"
[ "${#brew_lines[@]}" -gt 0 ] || fail "$homebrew lists no Homebrew package"

# The prefix's manifest, as build-nemo-speech.sh writes it (ink-engines' build.rs reads the same).
nemo_commit="" ggml_native=""
hashes=()
bundled=()
while IFS= read -r line; do
  read -r -a fields <<<"$line"
  case "${fields[0]-}" in
    '') continue ;;
    commit) [ "${#fields[@]}" = 2 ] || fail "$nemo: unreadable line '$line'"; nemo_commit="${fields[1]}"; continue ;;
    ggml_native) [ "${#fields[@]}" = 2 ] || fail "$nemo: unreadable line '$line'"; ggml_native="${fields[1]}"; continue ;;
    sha256)
      [ "${#fields[@]}" = 3 ] && [[ "${fields[1]}" =~ ^[0-9a-f]{64}$ ]] && [[ "${fields[2]}" =~ ^lib/[^/]+$ ]] \
        || fail "$nemo: unreadable line '$line'"
      hashes+=("${fields[1]} ${fields[2]}")
      continue
      ;;
    '#') [ "${fields[1]-}" = bundled ] || continue ;;
    '#'*) continue ;;
    *) fail "$nemo: unreadable line '$line'" ;;
  esac
  # "# bundled <name> <origin>": the name the app loads it by, and the file it was copied from.
  [ "${#fields[@]}" = 4 ] || fail "$nemo: unreadable line '$line'"
  name="${fields[2]}"
  origin="${fields[3]}"
  file="${origin##*/}"
  # Only the name in these messages: the path is the build machine's.
  [ "$origin" = "$deps_prefix/lib/$file" ] \
    || fail "$name was copied from outside the pinned SentencePiece/Abseil prefix: this record could give no version for it"
  entry=""
  for d in "${deps_libs[@]}"; do
    [ "${d%% *}" = "$file" ] && entry="${d#* }"
  done
  [ -n "$entry" ] || fail "$name was copied from lib/$file, which the pinned prefix's manifest does not list"
  [ -f "$origin" ] || fail "$name was copied from lib/$file, which is no longer in the pinned prefix"
  [ "$(sha256 "$origin")" = "${entry#* }" ] \
    || fail "$name was copied from lib/$file, which changed after the pinned prefix was built"
  project="${entry%% *}"
  bundled+=("$name $project $(version_of "$project") lib/$file")
done <"$nemo"
[ -n "$nemo_commit" ] || fail "$nemo records no commit"
[ -n "$ggml_native" ] || fail "$nemo records no ggml_native"
[ "${#hashes[@]}" -gt 0 ] || fail "$nemo lists no library"

tmp="$out.tmp.$$"
trap 'rm -f "$tmp"' EXIT
{
  echo "# Inkwell $version for the Mac: what its release build bundled (mac/scripts/build-manifest.sh)."
  echo "version $version"
  echo "source $commit"
  echo "dmg $(basename "$dmg") sha256 $(sha256 "$dmg")"
  echo "#"
  echo "# NeMo-Speech.cpp (the diarizer), built by core/crates/ink-engines/native/build-nemo-speech.sh:"
  echo "# its commit, its ggml's GGML_NATIVE, and each library's SHA-256 in the prefix it was installed"
  echo "# to. (The app signs its own copies again, so the files in the dmg hash differently.)"
  echo "nemo_speech commit $nemo_commit"
  echo "nemo_speech ggml_native $ggml_native"
  for h in "${hashes[@]}"; do echo "nemo_speech sha256 $h"; done
  echo "#"
  echo "# SentencePiece and Abseil, pinned: built by core/crates/ink-engines/native/"
  echo "# build-sentencepiece-abseil.sh from these release tarballs (project, version, SHA-256, file)."
  for p in "${pins[@]}"; do echo "pinned $p"; done
  echo "#"
  echo "# Bundled from them: the name the app loads, then the project, its version and the file that"
  echo "# was copied."
  for b in ${bundled[@]+"${bundled[@]}"}; do echo "bundled $b"; done
  echo "#"
  echo "# The build tools at build time (Homebrew's brew list --versions)."
  for entry in "${brew_lines[@]}"; do echo "homebrew $entry"; done
} >"$tmp"
# The last guard for a public file: no absolute path, whatever an input held.
if grep -nE '(^|[[:space:]])/' "$tmp" >&2; then
  fail "the manifest would name an absolute path (above)"
fi
mv "$tmp" "$out"
echo "build manifest: $out (${#bundled[@]} pinned librar$([ "${#bundled[@]}" = 1 ] && echo y || echo ies))"
