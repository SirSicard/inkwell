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

npx wrangler kv key put latest "$(cat "$TMP")" --binding INKWELL_RELEASES --remote
echo "Pushed. Verify: curl -s https://inkwell-updater.mattias-e67.workers.dev/api/update/darwin/aarch64/0.1.1"
