#!/usr/bin/env bash
# The build manifest a Mac release keeps beside its dmg: which third-party libraries the app
# bundles, where each came from, and the Homebrew versions the build used. The build log shows the
# same, but a log expires; a release asset lasts as long as the release, so a shipped dmg can
# always be traced to the SentencePiece and Abseil inside it. The release workflow attaches it to
# the draft release, and keeps it as the run's artifact (the dry run's only copy).
#
#   mac/scripts/build-manifest.sh --version X.Y.Z --commit <SHA-1> --dmg <dmg> \
#     --nemo <NeMo prefix>/share/inkwell/nemo-speech.manifest \
#     --homebrew <what `brew list --versions` printed> --out <file>
#
# The file is public, so it names no path on the build machine. build-nemo-speech.sh records
# where it copied each library from, a file in a Homebrew keg (<Homebrew>/Cellar/<formula>/
# <version>/...); that becomes the formula, the version and the file's place in the keg. An origin
# anywhere else is refused, not written: it has no version this record could give, and its path
# could be anyone's. Each keg's version must also be one Homebrew listed, so the two halves agree.
# Nothing is written unless all of it checks out.
set -euo pipefail

fail() { echo "build-manifest: $*" >&2; exit 1; }
usage="usage: build-manifest.sh --version X.Y.Z --commit <SHA-1> --dmg <dmg> --nemo <manifest> --homebrew <file> --out <file>"

version="" commit="" dmg="" nemo="" homebrew="" out=""
while [ $# -gt 0 ]; do
  case "$1" in
    --version | --commit | --dmg | --nemo | --homebrew | --out)
      [ $# -ge 2 ] || fail "$1 needs a value"
      case "$1" in
        --version) version="$2" ;;
        --commit) commit="$2" ;;
        --dmg) dmg="$2" ;;
        --nemo) nemo="$2" ;;
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
[ -f "$homebrew" ] || fail "no Homebrew version list at $homebrew"

# Homebrew's list: `<formula> <version>...`, one formula a line (a formula can have several kegs).
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
  if [[ ! "$origin" =~ ^(/opt/homebrew|/usr/local)/Cellar/([^/]+)/([^/]+)/(.+)$ ]]; then
    # Only the name: the path is the build machine's.
    fail "$name was copied from outside a Homebrew keg: this record could give no version for it"
  fi
  formula="${BASH_REMATCH[2]}"
  keg="${BASH_REMATCH[3]}"
  file="${BASH_REMATCH[4]}"
  listed=0
  for entry in "${brew_lines[@]}"; do
    [ "${entry%% *}" = "$formula" ] || continue
    case " ${entry#* } " in *" $keg "*) listed=1 ;; esac
  done
  [ "$listed" = 1 ] || fail "$name came from $formula $keg, which brew list --versions does not report"
  bundled+=("$name $formula $keg $file")
done <"$nemo"
[ -n "$nemo_commit" ] || fail "$nemo records no commit"
[ -n "$ggml_native" ] || fail "$nemo records no ggml_native"
[ "${#hashes[@]}" -gt 0 ] || fail "$nemo lists no library"

sha256() { shasum -a 256 <"$1" | cut -d' ' -f1; }
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
  echo "# Bundled from Homebrew: the name the app loads, then the formula, its version and the file"
  echo "# in its keg that was copied."
  for b in ${bundled[@]+"${bundled[@]}"}; do echo "bundled $b"; done
  echo "#"
  echo "# Homebrew at build time (brew list --versions)."
  for entry in "${brew_lines[@]}"; do echo "homebrew $entry"; done
} >"$tmp"
# The last guard for a public file: no absolute path, whatever an input held.
if grep -nE '(^|[[:space:]])/' "$tmp" >&2; then
  fail "the manifest would name an absolute path (above)"
fi
mv "$tmp" "$out"
echo "build manifest: $out (${#bundled[@]} librar$([ "${#bundled[@]}" = 1 ] && echo y || echo ies) from Homebrew)"
