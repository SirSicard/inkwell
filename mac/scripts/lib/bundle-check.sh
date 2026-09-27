# What an app bundle's code loads, and from where (sourced by build-mac.sh; tests:
# tests/test-bundle-check.sh).
#
# An app that works on the Mac that built it can still fail on a user's: a library loaded from
# Homebrew or /usr/local is simply not there on most Macs, an absolute rpath points at the build
# machine, and code built for a newer macOS than the app's target stops dyld on older ones. None of
# this shows until someone else starts the app, so every Mach-O in the bundle is read here instead.

# The load commands of a Mach-O, one record per line (every architecture of a universal file):
#   load <command> <name>   a library it loads (LC_LOAD_DYLIB, LC_LOAD_WEAK_DYLIB, ...)
#   id <name>               its own install name (a library)
#   rpath <path>            an LC_RPATH
#   minos <version>         the oldest macOS it runs on
#   type <filetype>         EXECUTE, DYLIB, BUNDLE, ...
bundle_check_records() {
  otool -hv "$1" | awk '$1 ~ /^MH_MAGIC/ { print "type", $5 }'
  otool -l "$1" | awk '
    $1 == "cmd" { cmd = $2; next }
    $1 == "name" && cmd == "LC_ID_DYLIB" { print "id", $2; next }
    $1 == "name" && cmd ~ /^LC_(LOAD|LOAD_WEAK|REEXPORT|LAZY_LOAD|LOAD_UPWARD)_DYLIB$/ { print "load", cmd, $2; next }
    $1 == "path" && cmd == "LC_RPATH" { print "rpath", $2; next }
    ($1 == "minos" && cmd == "LC_BUILD_VERSION") || ($1 == "version" && cmd == "LC_VERSION_MIN_MACOSX") { print "minos", $2 }'
}

# Whether dotted version $1 is newer than $2.
bundle_check_newer() {
  [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -t. -k1,1n -k2,2n -k3,3n | tail -1)" = "$1" ]
}

# check_bundle_linkage <app> <deployment target> [allow newer: 0 or 1]
#
# Fails (non-zero, each problem on stderr) unless every Mach-O in the bundle:
# - loads only system libraries (/usr/lib, /System/Library) by absolute path, and names no other
#   absolute path, its own install name included;
# - has only rpaths relative to itself or the executable (@loader_path, @executable_path);
# - finds every library it loads relative to itself (@rpath, @loader_path, @executable_path)
#   inside the bundle, through its own rpaths or, for a library, the app executable's (dyld's
#   search, simplified: a library loaded only by a helper resolves through that helper);
# - was built for the deployment target or older. With "allow newer", that last one only warns:
#   a local build on a Mac whose Homebrew was built for its own, newer macOS.
# And every link (a symlink) in the bundle, and every library a Mach-O loads, resolves, link by
# link to the last one, to a place inside the bundle. dyld follows links, and the app's signature
# seals a link as its target path, not as the file behind it: a link out of the bundle would load
# code no signature covers. A link that stays inside passes (a framework is built of them); what it
# points at is checked where it is.
check_bundle_linkage() {
  local app target allow main main_rpaths problems=0 checked=0 newer=0 f records rel dir kind exe own_rpaths candidates
  local rec cmd name rp resolved candidate found minos real
  app="$(cd "$1" && pwd -P)"
  target="$2"
  allow="${3:-0}"
  main="$app/Contents/MacOS/$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$app/Contents/Info.plist" 2>/dev/null || basename "$app" .app)"
  [ -f "$main" ] || main="$(find "$app/Contents/MacOS" -maxdepth 1 -type f | head -1)"
  main_rpaths="$(bundle_check_records "$main" | sed -n 's/^rpath //p')"

  problem() {
    echo "  $rel: $*" >&2
    problems=$((problems + 1))
  }

  while IFS= read -r -d '' f; do
    rel="${f#"$app/"}"
    if [ -L "$f" ]; then
      # realpath resolves every link on the way, the last one included.
      if ! real="$(realpath "$f" 2>/dev/null)"; then
        problem "a link to $(readlink "$f"), which does not exist"
        continue
      fi
      case "$real/" in
        "$app"/*) ;;
        *) problem "a link to $real, outside the bundle" ;;
      esac
      continue
    fi
    case "$(file -b "$f")" in Mach-O*) ;; *) continue ;; esac
    checked=$((checked + 1))
    dir="$(dirname "$f")"
    records="$(bundle_check_records "$f")"
    own_rpaths="$(sed -n 's/^rpath //p' <<<"$records")"
    kind="$(sed -n 's/^type //p' <<<"$records" | head -1)"
    if [ "$kind" = EXECUTE ]; then exe="$dir"; else exe="$(dirname "$main")"; fi

    while IFS= read -r rp; do
      case "$rp" in
        @loader_path | @loader_path/* | @executable_path | @executable_path/*) ;;
        *) problem "rpath $rp: an rpath must be relative to the code (@loader_path or @executable_path)" ;;
      esac
    done < <(sed -n 's/^rpath //p' <<<"$records")

    while IFS= read -r name; do
      case "$name" in
        /*) problem "its install name $name is an absolute path" ;;
      esac
    done < <(sed -n 's/^id //p' <<<"$records" | sort -u)

    while IFS=' ' read -r rec cmd name; do
      [ "$rec" = load ] || continue
      found=""
      case "$name" in
        /usr/lib/* | /System/Library/*) continue ;;
        /*)
          problem "loads $name, from outside the bundle and the OS"
          continue
          ;;
        @loader_path/*) found="$dir/${name#@loader_path/}" ;;
        @executable_path/*) found="$exe/${name#@executable_path/}" ;;
        @rpath/*)
          # Its own rpaths, then (a library) the app executable's, as dyld searches them. Built
          # into a variable first: the loop below leaves early (break), and when it read a process
          # substitution instead, the writer was left on a closed pipe. The release build died
          # here of a SIGTRAP, next to a broken pipe from that writer.
          candidates="$(
            while IFS= read -r rp; do
              if [ -n "$rp" ]; then printf '%s\t%s\n' "$dir" "$rp"; fi
            done <<<"$own_rpaths"
            if [ "$kind" != EXECUTE ]; then
              while IFS= read -r rp; do
                if [ -n "$rp" ]; then printf '%s\t%s\n' "$(dirname "$main")" "$rp"; fi
              done <<<"$main_rpaths"
            fi
          )"
          while IFS=$'\t' read -r candidate rp; do
            [ -n "$rp" ] || continue
            case "$rp" in
              @loader_path*) resolved="$candidate${rp#@loader_path}" ;;
              @executable_path*) resolved="$exe${rp#@executable_path}" ;;
              *) continue ;;
            esac
            if [ -e "$resolved/${name#@rpath/}" ]; then
              found="$resolved/${name#@rpath/}"
              break
            fi
          done <<<"$candidates"
          ;;
        *)
          problem "loads $name, which no rule here resolves"
          continue
          ;;
      esac
      if [ -z "$found" ] || [ ! -e "$found" ]; then
        # A weak library may be missing: dyld carries on without it.
        [ "$cmd" = LC_LOAD_WEAK_DYLIB ] && continue
        problem "loads $name, which does not resolve to a file in the bundle"
        continue
      fi
      # The file itself, not only its directory: the library may be a link.
      real="$(realpath "$found")"
      case "$real/" in
        "$app"/*) ;;
        *) problem "loads $name, which resolves outside the bundle ($real)" ;;
      esac
    done <<<"$records"

    while IFS= read -r minos; do
      if bundle_check_newer "$minos" "$target"; then
        if [ "$allow" = 1 ]; then
          newer=$((newer + 1))
          echo "  warning: $rel is built for macOS $minos, newer than the app's $target: this build will not start on older macOS" >&2
        else
          problem "built for macOS $minos, newer than the app's $target: it would stop the app on older macOS"
        fi
      fi
    done < <(sed -n 's/^minos //p' <<<"$records" | sort -u)
  done < <(find "$app/Contents" \( -type f -o -type l \) -print0)

  if [ "$problems" -ne 0 ]; then
    echo "  $problems problem(s) in what the bundle's code loads" >&2
    return 1
  fi
  if [ "$newer" -eq 0 ]; then
    echo "linkage: $checked Mach-O file(s) load only the OS and the bundle, by relative paths; built for macOS $target or older"
  else
    echo "linkage: $checked Mach-O file(s) load only the OS and the bundle, by relative paths; $newer built for a newer macOS than $target (allowed: a local build)"
  fi
}
