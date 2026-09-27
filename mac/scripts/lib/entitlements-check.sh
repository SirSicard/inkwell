# The app's signed entitlements against an allow-list (sourced by build-mac.sh; tests:
# tests/test-entitlements-check.sh).
#
# The set signed into the app must be exactly mac/Inkwell.entitlements, key for key and value for
# value, plus com.apple.security.cs.disable-library-validation (true) for an ad-hoc build and never
# for a Developer ID one. A missing key is a feature the hardened runtime denies silently (the
# microphone, the calendar: no prompt, the grant reads as denied); an extra one is a capability
# nobody reviewed. Entitlements are a flat dictionary, which is all this reads.

# entitlement_keys <plist>: its top-level keys, sorted, one per line. Fails for an unreadable file.
entitlement_keys() {
  local xml
  xml="$(plutil -convert xml1 -o - "$1" 2>/dev/null)" || return 1
  sed -n 's:.*<key>\(.*\)</key>.*:\1:p' <<<"$xml" | sort -u
}

# entitlement_type <plist> <key>: the top-level value's plist type (true, false, string, integer,
# array, dict, ...), read from the element after the key in plutil's XML, where a top-level key is
# indented by one tab.
entitlement_type() {
  plutil -convert xml1 -o - "$1" 2>/dev/null | awk -v key="	<key>$2</key>" '
    found { if (match($0, /<[a-z]+/)) print substr($0, RSTART + 1, RLENGTH - 1); exit }
    $0 == key { found = 1 }'
}

# entitlement_value <plist> <key>: "true" or "false" for a boolean, otherwise its type and the value
# as PlistBuddy prints it. PlistBuddy alone prints <true/> and <string>true</string> alike, and the
# hardened runtime honours only the boolean. (PlistBuddy, not plutil, for the value: plutil's key
# paths split on the dots in the key.)
entitlement_value() {
  local type
  type="$(entitlement_type "$1" "$2")"
  case "$type" in
    true | false) echo "$type" ;;
    *) echo "$type $(/usr/libexec/PlistBuddy -c "Print :$2" "$1" 2>/dev/null)" ;;
  esac
}

# check_entitlements <signed plist> <expected plist> <adhoc: 1 or 0>
# Returns 0 when the signed set is exactly the expected one; otherwise prints each difference and
# returns 1.
check_entitlements() {
  local signed="$1" expected="$2" adhoc="$3"
  local dlv=com.apple.security.cs.disable-library-validation
  local want have key value expected_value problems=0
  want="$(entitlement_keys "$expected")" || { echo "entitlements: $expected cannot be read"; return 1; }
  have="$(entitlement_keys "$signed")" || { echo "entitlements: the signed entitlements cannot be read"; return 1; }
  if [ "$adhoc" = 1 ]; then
    want="$(printf '%s\n%s\n' "$want" "$dlv" | sed '/^$/d' | sort -u)"
  fi
  while IFS= read -r key; do
    [ -n "$key" ] || continue
    if ! grep -qxF "$key" <<<"$have"; then
      echo "entitlements: $key is missing"
      problems=1
      continue
    fi
    value="$(entitlement_value "$signed" "$key")"
    if [ "$key" = "$dlv" ] && [ "$adhoc" = 1 ]; then
      expected_value=true
    else
      expected_value="$(entitlement_value "$expected" "$key")"
    fi
    if [ "$value" != "$expected_value" ]; then
      echo "entitlements: $key is $value, expected $expected_value"
      problems=1
    fi
  done <<<"$want"
  while IFS= read -r key; do
    [ -n "$key" ] || continue
    if ! grep -qxF "$key" <<<"$want"; then
      echo "entitlements: $key was not asked for"
      problems=1
    fi
  done <<<"$have"
  return "$problems"
}
