#!/usr/bin/env bash
# The Mac capture checklist (S2.1a): steps 1 to 5 of
# core/crates/ink-platform-mac/CAPTURE-CHECKLIST.md, which says what each step must show.
#
# Run it by hand, in a terminal app that has Microphone and System Audio Recording (System
# Settings > Privacy & Security). An agent's shell cannot: capture needs those grants, and the
# first tone probe may show macOS's System Audio prompt.
set -euo pipefail

cd "$(dirname "$0")/../core"
cargo build --quiet -p ink-platform-mac --example capture_check
check=./target/debug/examples/capture_check

failed=()
step() {
  local name=$1
  shift
  printf '\n== %s\n' "$name"
  if "$check" "$@"; then
    printf -- '-- %s: ok\n' "$name"
  else
    printf -- '-- %s: FAILED\n' "$name"
    failed+=("$name")
  fi
}
pause() {
  printf '\n%s\n' "$1"
  read -r -p "Press Enter when ready. " _
}

step "1 devices and routing" --devices
step "2 permissions and the tone probe" --permissions --probe
pause "3: stop everything that plays sound (music, videos, calls)."
step "3 idle far end" --capture 10 --expect-idle-far
pause "4: open https://zoom.us/test and join the test meeting. Speak when it asks you to, and
leave the meeting before the 60 s are up."
step "4 zoom test call" --capture 60
pause "5: in System Settings > Privacy & Security > Screen & System Audio Recording, switch this
terminal OFF under System Audio Recording Only. (If step 5 still says Granted, quit and reopen the
terminal and run: $check --permissions --probe)"
step "5 system audio revoked" --permissions --probe
pause "Switch this terminal back ON under System Audio Recording Only."

printf '\n'
if ((${#failed[@]} == 0)); then
  echo "All steps exited ok. Step 5 must show 'Silence -> Denied': check it above."
else
  echo "Failed: ${failed[*]}"
  echo "(Step 5 exits ok even when it shows Denied; that is the expected result there.)"
  exit 1
fi
