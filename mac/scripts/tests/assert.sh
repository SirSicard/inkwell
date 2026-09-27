# Assertions for the script tests (sourced). Each counts; `finish` exits non-zero if any failed.
failures=0

pass() { printf '  ok    %s\n' "$1"; }
flunk() { printf '  FAIL  %s\n' "$1"; failures=$((failures + 1)); }

# assert_contains <label> <haystack> <needle>
assert_contains() {
  case "$2" in
    *"$3"*) pass "$1" ;;
    *) flunk "$1: expected to find [$3] in:"; printf '%s\n' "$2" | sed 's/^/        /' ;;
  esac
}

# assert_absent <label> <haystack> <needle>
assert_absent() {
  case "$2" in
    *"$3"*) flunk "$1: [$3] survived in:"; printf '%s\n' "$2" | sed 's/^/        /' ;;
    *) pass "$1" ;;
  esac
}

# assert_status <label> <expected> <actual>
assert_status() {
  if [ "$2" = "$3" ]; then pass "$1"; else flunk "$1: exit status $3, expected $2"; fi
}

finish() {
  if [ "$failures" -ne 0 ]; then
    echo "  $failures assertion(s) failed"
    exit 1
  fi
}
