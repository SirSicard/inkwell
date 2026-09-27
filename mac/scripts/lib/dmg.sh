# Mounting a release dmg to read it (sourced by verify-release.sh and appcast.sh).
#
# Read-only, hidden from the Finder, never auto-opened: these scripts only read what is inside.
# (hdiutil prints a deprecation notice for -mountpoint on newer macOS; -quiet keeps it out of the
# logs, and the exit status still reports a failure.)

# dmg_attach <dmg> <mountpoint>: mounts it there, creating the directory.
dmg_attach() {
  mkdir -p "$2"
  hdiutil attach -quiet -readonly -nobrowse -noautoopen -mountpoint "$2" "$1"
}

# dmg_detach <mountpoint>: unmounts it if mounted; forced after a first refusal (a Spotlight or
# Gatekeeper scan can hold the volume for a moment).
dmg_detach() {
  local point mounts
  [ -d "$1" ] || return 0
  # mount(8) prints the resolved path (/private/tmp, not /tmp). Read whole before matching: grep -q
  # closing a pipe early would fail it under pipefail and skip the detach.
  point="$(cd "$1" && pwd -P)"
  mounts="$(mount)"
  grep -qF " on $point (" <<<"$mounts" || return 0
  hdiutil detach -quiet "$point" 2>/dev/null || hdiutil detach -quiet -force "$point"
}
