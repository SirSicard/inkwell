#!/usr/bin/env bash
# The tests of mac/scripts: every test-*.sh here, each on its own. Offline; the licence audit's
# tests need `swift` (they read fixture manifests with `swift package dump-package`).
#
#   mac/scripts/tests/run.sh [test-name.sh ...]
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
if [ $# -eq 0 ]; then
  set -- "$here"/test-*.sh
fi
failed=()
for test in "$@"; do
  name="$(basename "$test")"
  printf '== %s\n' "$name"
  if /bin/bash "$here/$name"; then
    printf -- '-- %s: ok\n' "$name"
  else
    printf -- '-- %s: FAILED\n' "$name"
    failed+=("$name")
  fi
done
echo
if [ ${#failed[@]} -ne 0 ]; then
  echo "failed: ${failed[*]}"
  exit 1
fi
echo "all script tests passed"
