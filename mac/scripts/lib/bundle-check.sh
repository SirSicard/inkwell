# What an app bundle's code loads, and from where (sourced by build-mac.sh; tests:
# tests/test-bundle-check.sh).
#
# An app that works on the Mac that built it can still fail on a user's: a library loaded from
# Homebrew or /usr/local is simply not there on most Macs, an absolute rpath points at the build
# machine, and code built for a newer macOS than the app's target stops dyld on older ones. None of
# this shows until someone else starts the app, so every Mach-O in the bundle is read here instead.

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
#
# The work is bundle-check.py's, in one process. It was bash until the release bundle (95 Mach-O
# files: the engines' libraries) outgrew macOS's bash 3.2: thousands of command substitutions and
# here-strings corrupted its heap, and the build died of a SIGTRAP in a forked child, or a
# substitution came back empty and the check misread a file: a library beside its loader reported
# missing, or (run alone) 11 of the 95 files skipped as not Mach-O, and the check passed.
check_bundle_linkage() {
  python3 "$(dirname "${BASH_SOURCE[0]}")/bundle-check.py" "$@"
}
