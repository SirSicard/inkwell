"""What an app bundle's code loads, and from where: the work of check_bundle_linkage, which
lib/bundle-check.sh defines, with the rules and why this is Python.

    python3 bundle-check.py <app> <deployment target> [allow newer: 0 or 1]

Each problem goes to stderr, then their count, and the exit status is 1. Otherwise one summary line
goes to stdout. Standard library only; otool and file run once per file, never once per line.
"""

import os
import plistlib
import re
import subprocess
import sys

LOAD_COMMANDS = re.compile(r"LC_(LOAD|LOAD_WEAK|REEXPORT|LAZY_LOAD|LOAD_UPWARD)_DYLIB")


def output(args):
    """A tool's stdout, or None when it fails (its own message is on stderr)."""
    done = subprocess.run(args, stdout=subprocess.PIPE, encoding="utf-8", errors="surrogateescape")
    return done.stdout if done.returncode == 0 else None


def field(fields, i):
    """awk's $i: an empty string past the last field."""
    return fields[i] if i < len(fields) else ""


def records(path):
    """The load commands of a Mach-O, every architecture of a universal file, in otool's order:
    "type" (EXECUTE, DYLIB, BUNDLE, ...), "id" (its own install name, a library's), "load"
    ((command, name) for LC_LOAD_DYLIB, LC_LOAD_WEAK_DYLIB, ...), "rpath" (each LC_RPATH) and
    "minos" (the oldest macOS it runs on). None when otool cannot read it."""
    header = output(["otool", "-hv", path])
    commands = output(["otool", "-l", path])
    if header is None or commands is None:
        return None
    found = {"type": [], "id": [], "load": [], "rpath": [], "minos": []}
    for line in header.splitlines():
        fields = line.split()
        if fields and fields[0].startswith("MH_MAGIC"):
            found["type"].append(field(fields, 4))
    cmd = ""
    for line in commands.splitlines():
        fields = line.split()
        key, value = field(fields, 0), field(fields, 1)
        if key == "cmd":
            cmd = value
        elif key == "name" and cmd == "LC_ID_DYLIB":
            found["id"].append(value)
        elif key == "name" and LOAD_COMMANDS.fullmatch(cmd):
            found["load"].append((cmd, value))
        elif key == "path" and cmd == "LC_RPATH":
            found["rpath"].append(value)
        elif (key == "minos" and cmd == "LC_BUILD_VERSION") or (
            key == "version" and cmd == "LC_VERSION_MIN_MACOSX"
        ):
            found["minos"].append(value)
    return found


def version_key(version):
    """A dotted version as the check has always ordered it (sort -t. -k1,1n -k2,2n -k3,3n): its
    first three fields as numbers, an empty or non-numeric one as 0, then the whole text."""
    fields = version.split(".")
    numbers = []
    for i in range(3):
        number = re.match(r"\s*(-?\d+)", field(fields, i))
        numbers.append(int(number.group(1)) if number else 0)
    return numbers, version.encode("utf-8", "surrogateescape")


def newer(version, than):
    return version != than and version_key(version) > version_key(than)


def entries(path):
    """(path, is a link) for every file and link at or under path, in directory order, links not
    followed: what `find path \\( -type f -o -type l \\)` lists."""
    if os.path.islink(path):
        yield path, True
    elif os.path.isdir(path):
        with os.scandir(path) as listing:
            children = [entry.path for entry in listing]
        for child in children:
            yield from entries(child)
    elif os.path.isfile(path):
        yield path, False


def main_executable(app):
    """The app's executable: CFBundleExecutable, else the bundle's name, else the first file in
    Contents/MacOS; "" when there is none."""
    try:
        with open(os.path.join(app, "Contents", "Info.plist"), "rb") as plist:
            name = plistlib.load(plist)["CFBundleExecutable"]
    except Exception:  # any Info.plist that does not name it: the fallbacks below say which file
        name = None
    if not isinstance(name, str):
        name = os.path.basename(app)
        if name.endswith(".app") and name != ".app":
            name = name[: -len(".app")]
    main = os.path.join(app, "Contents", "MacOS", name)
    if os.path.isfile(main):
        return main
    macos = os.path.join(app, "Contents", "MacOS")
    if os.path.isdir(macos):
        with os.scandir(macos) as listing:
            for entry in listing:
                if entry.is_file(follow_symlinks=False):
                    return entry.path
    return ""


def check(app, target, allow):
    app = os.path.realpath(app)
    main = main_executable(app)
    main_dir = os.path.dirname(main)
    main_records = records(main) if main else None
    main_rpaths = main_records["rpath"] if main_records else []
    problems = 0
    checked = 0
    newer_count = 0

    def problem(rel, message):
        nonlocal problems
        print(f"  {rel}: {message}", file=sys.stderr)
        problems += 1

    def inside(real):
        return (real + "/").startswith(app + "/")

    for path, is_link in entries(os.path.join(app, "Contents")):
        rel = path[len(app) + 1 :]
        if is_link:
            # Every link on the way, the last one included.
            real = os.path.realpath(path)
            if not os.path.exists(real):
                problem(rel, f"a link to {os.readlink(path)}, which does not exist")
            elif not inside(real):
                problem(rel, f"a link to {real}, outside the bundle")
            continue
        kind_of_file = output(["file", "-b", path])
        if kind_of_file is None:
            problem(rel, "couldn't read what kind of file it is (file failed)")
            continue
        if not kind_of_file.startswith("Mach-O"):
            continue
        checked += 1
        directory = os.path.dirname(path)
        found_records = records(path)
        if found_records is None:
            problem(rel, "couldn't read its load commands (otool failed)")
            continue
        kind = field(found_records["type"], 0)
        exe = directory if kind == "EXECUTE" else main_dir

        for rp in found_records["rpath"]:
            if not re.fullmatch(r"@loader_path(/.*)?|@executable_path(/.*)?", rp, re.DOTALL):
                problem(
                    rel,
                    f"rpath {rp}: an rpath must be relative to the code (@loader_path or @executable_path)",
                )

        for name in sorted(set(found_records["id"])):
            if name.startswith("/"):
                problem(rel, f"its install name {name} is an absolute path")

        for cmd, name in found_records["load"]:
            found = ""
            if name.startswith("/usr/lib/") or name.startswith("/System/Library/"):
                continue
            if name.startswith("/"):
                problem(rel, f"loads {name}, from outside the bundle and the OS")
                continue
            if name.startswith("@loader_path/"):
                found = directory + "/" + name[len("@loader_path/") :]
            elif name.startswith("@executable_path/"):
                found = exe + "/" + name[len("@executable_path/") :]
            elif name.startswith("@rpath/"):
                # Its own rpaths, then (a library) the app executable's, as dyld searches them.
                searched = [(directory, rp) for rp in found_records["rpath"] if rp]
                if kind != "EXECUTE":
                    searched += [(main_dir, rp) for rp in main_rpaths if rp]
                for base, rp in searched:
                    if rp.startswith("@loader_path"):
                        resolved = base + rp[len("@loader_path") :]
                    elif rp.startswith("@executable_path"):
                        resolved = exe + rp[len("@executable_path") :]
                    else:
                        continue
                    candidate = resolved + "/" + name[len("@rpath/") :]
                    if os.path.exists(candidate):
                        found = candidate
                        break
            else:
                problem(rel, f"loads {name}, which no rule here resolves")
                continue
            if not found or not os.path.exists(found):
                # A weak library may be missing: dyld carries on without it.
                if cmd == "LC_LOAD_WEAK_DYLIB":
                    continue
                problem(rel, f"loads {name}, which does not resolve to a file in the bundle")
                continue
            # The file itself, not only its directory: the library may be a link.
            real = os.path.realpath(found)
            if not inside(real):
                problem(rel, f"loads {name}, which resolves outside the bundle ({real})")

        for minos in sorted(set(found_records["minos"])):
            if newer(minos, target):
                if allow == "1":
                    newer_count += 1
                    print(
                        f"  warning: {rel} is built for macOS {minos}, newer than the app's {target}: "
                        "this build will not start on older macOS",
                        file=sys.stderr,
                    )
                else:
                    problem(
                        rel,
                        f"built for macOS {minos}, newer than the app's {target}: it would stop the app on older macOS",
                    )

    if problems:
        print(f"  {problems} problem(s) in what the bundle's code loads", file=sys.stderr)
        return 1
    if newer_count == 0:
        print(
            f"linkage: {checked} Mach-O file(s) load only the OS and the bundle, by relative paths; "
            f"built for macOS {target} or older"
        )
    else:
        print(
            f"linkage: {checked} Mach-O file(s) load only the OS and the bundle, by relative paths; "
            f"{newer_count} built for a newer macOS than {target} (allowed: a local build)"
        )
    return 0


def main(argv):
    if len(argv) not in (3, 4):
        print("usage: bundle-check.py <app> <deployment target> [allow newer: 0 or 1]", file=sys.stderr)
        return 2
    if not os.path.isdir(argv[1]):
        print(f"  couldn't check {argv[1]}: not a directory", file=sys.stderr)
        return 1
    allow = argv[3] if len(argv) == 4 and argv[3] else "0"
    return check(argv[1], argv[2], allow)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
