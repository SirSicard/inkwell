# Signing details made safe to print (sourced by build-mac.sh; tests: tests/test-redact-signing.sh).
#
# A Developer ID certificate's name is "Developer ID Application: <legal name> (<team ID>)", and
# for an individual account the legal name is a person's. Nothing build-mac.sh prints may carry
# it, the team ID, or the identity it was given. Requirements keep their structure, so a
# mismatch can still be read.

# redact_signing [literal ...] < text
# Replaces each literal (the identity as given: a SHA-1 hash or a name) with <identity>, every
# certificate name with <certificate name>, subject organisations with <name>, and every team ID
# (ten capitals and digits, as a whole word) with <team>.
redact_signing() {
  local line literal
  while IFS= read -r line || [ -n "$line" ]; do
    for literal in "$@"; do
      if [ -n "$literal" ]; then
        line="${line//"$literal"/<identity>}"
      fi
    done
    printf '%s\n' "$line"
  done | sed -E \
    -e 's/(Developer ID (Application|Installer)|Apple (Development|Distribution)|Mac Developer|3rd Party Mac Developer (Application|Installer)|Mac App Distribution|iPhone (Developer|Distribution)): [^"()]*(\([A-Z0-9]{10}\))?/<certificate name>/g' \
    -e 's/(subject\.CN\] = )"[^"]*"/\1"<certificate name>"/g' \
    -e 's/(subject\.CN\] = )[^" ][^ ]*/\1<certificate name>/g' \
    -e 's/(subject\.O\] = )("[^"]*"|[^" ][^ ]*)/\1<name>/g' \
    -e 's/(subject\.OU\] = )("[^"]*"|[^" ][^ ]*)/\1<team>/g' \
    -e 's/^(TeamIdentifier=).*/\1<team>/' \
    -e 's/[[:<:]][A-Z0-9]{10}[[:>:]]/<team>/g'
}
