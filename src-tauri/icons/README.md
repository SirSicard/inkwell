# App icons

`icon-1024.png` is the master. Everything else here is generated from it:

```bash
cargo tauri icon src-tauri/icons/icon-1024.png
```

`tray-template.png` is drawn separately, by hand, for 22pt. Do not generate it
from the app icon: macOS flattens a menu-bar icon to a silhouette, so the app
icon becomes an unreadable blob at that size.

## Why this icon

A cream drop with a dark navy highlight, chosen over six darker candidates in
0.1.1 because it stays visible on dark docks and taskbars, which the dark
variants do not. The rejected candidates were removed from the working tree in
0.2.3: they were about 11 MB of drafts that every clone paid for, and one of
them was a byte-identical copy of this master. They remain in git history if a
rebrand ever wants them (`git log --all -- icon-options/`).
