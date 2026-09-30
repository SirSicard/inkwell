#!/bin/sh
# Push a 0.2 release's latest.json into the updater's KV.
#
# The worker serves updates from KV, not from GitHub, so publishing a release
# does nothing for installed copies until this runs. Run it after every 0.2
# release, naming its tag. The tag is not read from releases/latest: 1.x
# releases carry no latest.json, and 1.x is the release marked latest.
# Requires wrangler to be logged in (npx wrangler login).
#
#   inkwell-updater/publish-latest.sh v0.2.11
set -e
cd "$(dirname "$0")"

TAG="${1:-}"
case "$TAG" in
  v0.2.[0-9] | v0.2.[0-9][0-9] | v0.2.[0-9][0-9][0-9]) ;;
  *)
    echo "usage: $0 v0.2.N   (the 0.2 release whose latest.json to publish)" >&2
    exit 2
    ;;
esac

TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT

curl -sfL "https://github.com/SirSicard/inkwell/releases/download/$TAG/latest.json" -o "$TMP"

# Refuse to push something that is not JSON (a GitHub error page, an empty
# body): a malformed KV value makes the worker answer 500 to every client.
python3 -m json.tool "$TMP" > /dev/null

# The put has failed transiently twice (0.2.6 and 0.2.8), both times printing
# an account-permissions dump and exiting while a direct rerun succeeded. So:
# one retry, and then trust nothing until the value read back matches what was
# pushed. A release whose manifest silently stays on the old version strands
# every installed copy with no error anywhere.
WANT=$(python3 -c "import json;print(json.load(open('$TMP'))['version'])")
# The manifest must be the named release's own: a typo'd or reused tag would
# otherwise push another version under this one's name. (The worker reads the
# version with or without a leading v.)
if [ "${WANT#v}" != "${TAG#v}" ]; then
  echo "FAILED: $TAG's latest.json names version $WANT" >&2
  exit 1
fi
for attempt in 1 2; do
  if npx wrangler kv key put latest "$(cat "$TMP")" --binding INKWELL_RELEASES --remote; then
    break
  fi
  echo "put failed (attempt $attempt)"; sleep 3
done
GOT=$(npx wrangler kv key get latest --binding INKWELL_RELEASES --remote 2>/dev/null   | python3 -c "import sys,json;print(json.load(sys.stdin)['version'])" || echo "unreadable")
if [ "$GOT" != "$WANT" ]; then
  echo "FAILED: KV reads back $GOT, expected $WANT" >&2
  exit 1
fi
echo "Pushed and verified: KV serves $GOT"
