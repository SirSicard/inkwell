# App icons

`icon-1024.png` is the master. Everything else here is generated from it:

```bash
cargo tauri icon src-tauri/icons/icon-1024.png
```

`tray-template.png` is drawn separately, by hand, for 22pt. Do not generate it
from the app icon: macOS flattens a menu-bar icon to a silhouette, so the app
icon becomes an unreadable blob at that size.

## Why this icon

A cream ink drop over two ripples, on charcoal with a warm bloom behind it.

It replaced a cream drop on a **blue-purple gradient** in 0.2.10. That gradient
appeared nowhere else in the product: the site, the recording overlay and the
app's own ink panel are all charcoal, cream and `#c8956c`, so the one asset a
user sees every day was the one thing off-brand. The drop itself also filled
only about half the canvas, and macOS crops the corners into a squircle, so
what reached the Dock was a small mark floating in a large field.

The ripples are drawn with a gradient stroke that fades at both ends. Drawn as
plain closed ellipses they read as two grey rings sitting on top of the icon
rather than as water, which is what the first attempt looked like at 1024.

Checked at the size it is actually used: at 32x32 the drop stays crisp and the
ripples resample to a soft dash rather than disappearing, and the tile still
reads against both a dark and a light dock.

`icon.svg` is the vector source and is what to edit. The previous master was a
PNG with no source, so changing it meant redrawing from pixels.

Do not commit the `android/` and `ios/` directories that `tauri icon` also
emits. This project does not ship mobile (see TODO.md, "Not doing"), and an
earlier round of unused icon assets cost every clone 11 MB before being removed.
