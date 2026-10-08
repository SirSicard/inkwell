# Remove only SwiftPM's build-local framework search path from the copied executable.
# All other paths remain for check_bundle_linkage to validate before signing.
remove_swiftpm_framework_rpath() {
  local executable=$1 mac_dir=$2 bin_dir=$3 candidate paths physical_build physical_bin
  case "$bin_dir" in
    "$mac_dir/.build/"*) ;;
    *) echo "SwiftPM binary directory is outside the package build directory" >&2; return 1 ;;
  esac
  physical_build="$(cd "$mac_dir/.build" && pwd -P)" || return 1
  physical_bin="$(cd "$bin_dir" && pwd -P)" || return 1
  case "$physical_bin" in
    "$physical_build/"*) ;;
    *) echo "SwiftPM binary directory resolves outside the package build directory" >&2; return 1 ;;
  esac
  candidate="$bin_dir/PackageFrameworks"
  paths="$(otool -l "$executable" | awk '
    $1 == "cmd" { rpath = ($2 == "LC_RPATH") }
    rpath && $1 == "path" {
      line = $0
      sub(/^[[:space:]]*path /, "", line)
      sub(/ \(offset [0-9]+\)$/, "", line)
      print line
      rpath = 0
    }')" || return 1
  if grep -Fxq -- "$candidate" <<<"$paths"; then
    [ -d "$candidate" ] || { echo "SwiftPM PackageFrameworks directory is missing" >&2; return 1; }
    install_name_tool -delete_rpath "$candidate" "$executable" || return 1
  fi
}
