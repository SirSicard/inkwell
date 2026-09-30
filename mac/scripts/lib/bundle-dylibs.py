"""Copies the libraries an app loads as @rpath/<name> into its Contents/Frameworks, then whatever
each loads through @rpath in turn: the work of bundle_dylib in build-mac.sh, which holds the rules
(the names allowed) and says why this is Python.

    python3 bundle-dylibs.py --into <Frameworks directory> [--allow <pattern>]...
        [--from <directory>]... -- <name>...

A name is copied only if it matches an --allow pattern (a shell pattern, as in a `case`), from the
first --from directory that holds it, under that name (a link there is followed: the file goes in).
Standard library only; otool and cp run once per library, never once per line.
"""

import argparse
import fnmatch
import os
import re
import stat
import subprocess
import sys

# otool -L's lines after the first (the file's own path): "<tab><name> (compatibility ...)".
LOADED = re.compile(r"[ \t\n\r\f\v]+([^ ]+) \(")


def fail(message):
    print(f"build-mac: {message}", file=sys.stderr)
    sys.exit(1)


def rpath_dylibs(path):
    """The names of the libraries path loads as @rpath/<name>.dylib, in otool's order."""
    done = subprocess.run(
        ["otool", "-L", path], stdout=subprocess.PIPE, encoding="utf-8", errors="surrogateescape"
    )
    if done.returncode != 0:
        fail(f"couldn't read what {os.path.basename(path)} loads (otool failed)")
    names = []
    for line in done.stdout.splitlines()[1:]:
        loaded = LOADED.match(line)
        if loaded and fnmatch.fnmatchcase(loaded.group(1), "@rpath/*.dylib"):
            names.append(loaded.group(1)[len("@rpath/") :])
    return names


def main(argv):
    parser = argparse.ArgumentParser(description="Bundles the libraries an app loads through @rpath.")
    parser.add_argument("--into", dest="frameworks", required=True)
    parser.add_argument("--allow", action="append", default=[])
    parser.add_argument("--from", dest="dirs", action="append", default=[])
    parser.add_argument("names", nargs="*")
    args = parser.parse_args(argv[1:])

    def bundle(name):
        dest = os.path.join(args.frameworks, name)
        if os.path.exists(dest):
            return
        if not any(fnmatch.fnmatchcase(name, pattern) for pattern in args.allow):
            fail(
                f"the app loads @rpath/{name}, which THIRD_PARTY.md does not cover: "
                "read its licence and list it first"
            )
        src = next((f"{d}/{name}" for d in args.dirs if os.path.exists(f"{d}/{name}")), None)
        if src is None:
            fail(f"the app loads @rpath/{name}, which no library directory of the core holds")
        os.makedirs(args.frameworks, exist_ok=True)
        # cp, as before: the file behind a link, with its mode and extended attributes.
        if subprocess.run(["cp", src, dest]).returncode != 0:
            fail(f"couldn't copy {name} into the bundle (cp failed)")
        os.chmod(dest, os.stat(dest).st_mode | stat.S_IWUSR)
        for loaded in rpath_dylibs(dest):
            bundle(loaded)

    for name in args.names:
        bundle(name)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
