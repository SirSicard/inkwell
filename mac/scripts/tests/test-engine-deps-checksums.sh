#!/usr/bin/env bash
# build-sentencepiece-abseil.sh: a tarball whose SHA-256 is not the pinned one is refused before
# anything is extracted or built, whether it was already in the downloads or was just fetched.
# Offline: `curl` is a stand-in that writes a wrong file, and nothing is built.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/assert.sh"
script="$here/../../../core/crates/ink-engines/native/build-sentencepiece-abseil.sh"
pin() { sed -n "s/^$1=//p" "$script"; }
absl_tarball="$(pin ABSEIL_TARBALL)"
absl_sha="$(pin ABSEIL_SHA256)"
spm_tarball="$(pin SENTENCEPIECE_TARBALL)"

work="$(mktemp -d "${TMPDIR:-/tmp}/ink-engine-deps-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT

# A curl that fetches nothing: it writes a wrong file where it was asked to, and logs the call.
mkdir -p "$work/bin"
cat >"$work/bin/curl" <<'EOF'
#!/usr/bin/env bash
echo "curl $*" >>"$CURL_LOG"
while [ $# -gt 0 ]; do
  if [ "$1" = --output ]; then printf 'not the release\n' >"$2"; fi
  shift
done
EOF
chmod +x "$work/bin/curl"
export CURL_LOG="$work/curl.log"

# run <label> <expected status> <case dir>: sets $out.
run() {
  local label=$1 want=$2 dir=$3 status=0
  out="$(PATH="$work/bin:$PATH" /bin/bash "$script" "$dir/prefix" "$dir/work" 2>&1)" || status=$?
  assert_status "$label" "$want" "$status"
}

# A tarball already in the downloads, with other contents.
c="$work/present"
mkdir -p "$c/work/downloads"
printf 'tampered\n' >"$c/work/downloads/$absl_tarball"
: >"$CURL_LOG"
run "a tarball in the downloads that is not the pinned one" 1 "$c"
assert_contains "... says why" "$out" "not the pinned $absl_sha: refused"
assert_contains "... and what to do" "$out" "delete it to fetch it again"
[ -f "$c/work/downloads/$absl_tarball" ] && pass "... leaves the file to look at" || flunk "... deleted the file"
[ ! -s "$CURL_LOG" ] && pass "... fetches nothing" || flunk "... called curl: $(cat "$CURL_LOG")"
[ ! -e "$c/work/src" ] && pass "... extracts nothing" || flunk "... extracted into $c/work/src"
[ ! -e "$c/prefix" ] && pass "... installs nothing" || flunk "... created $c/prefix"

# A download that is not the pinned tarball.
c="$work/fetched"
mkdir -p "$c"
: >"$CURL_LOG"
run "a download that is not the pinned tarball" 1 "$c"
assert_contains "... says why" "$out" "$absl_tarball from https://"
assert_contains "... and that it is gone" "$out" "not the pinned $absl_sha: refused (deleted)"
assert_contains "... fetched from the pinned URL" "$(cat "$CURL_LOG")" " $(pin ABSEIL_URL)"
assert_contains "... over https only" "$(cat "$CURL_LOG")" "--proto =https --proto-redir =https"
[ ! -e "$c/work/downloads/$absl_tarball" ] && [ ! -e "$c/work/downloads/$absl_tarball.part" ] \
  && pass "... keeps no copy of it" || flunk "... left the download in $c/work/downloads"
[ ! -e "$c/work/downloads/$spm_tarball" ] && pass "... and stops there" || flunk "... went on to fetch $spm_tarball"
[ ! -e "$c/work/src" ] && pass "... extracts nothing" || flunk "... extracted into $c/work/src"

# An earlier install is never built over, and that is checked before any fetch.
c="$work/used"
mkdir -p "$c/prefix/lib"
: >"$CURL_LOG"
run "a prefix that is not empty" 1 "$c"
assert_contains "... says why" "$out" "is not empty: install into a new prefix"
[ ! -s "$CURL_LOG" ] && pass "... before fetching anything" || flunk "... called curl: $(cat "$CURL_LOG")"

out="$(/bin/bash "$script" 2>&1)" && status=0 || status=$?
assert_status "a call without its two arguments" 2 "$status"
assert_contains "... prints the usage" "$out" "build-sentencepiece-abseil.sh <prefix> <work dir>"

finish
