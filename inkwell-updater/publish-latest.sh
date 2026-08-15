#!/bin/sh
# Push the newest GitHub release's latest.json into the updater's KV.
#
# The worker serves updates from KV, not from GitHub, so publishing a release
# does nothing for installed copies until this runs. Run it after every
# release. Requires wrangler to be logged in (npx wrangler login).
set -e
cd "$(dirname "$0")"

TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT

curl -sfL "https://github.com/SirSicard/inkwell/releases/latest/download/latest.json" -o "$TMP"

# Refuse to push something that is not JSON (a GitHub error page, an empty
# body): a malformed KV value makes the worker answer 500 to every client.
python3 -m json.tool "$TMP" > /dev/null

# The put has failed transiently twice (0.2.6 and 0.2.8), both times printing
# an account-permissions dump and exiting while a direct rerun succeeded. So:
# one retry, and then trust nothing until the value read back matches what was
# pushed. A release whose manifest silently stays on the old version strands
# every installed copy with no error anywhere.
WANT=$(python3 -c "import json;print(json.load(open('$TMP'))['version'])")
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
