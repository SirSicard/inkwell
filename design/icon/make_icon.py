#!/usr/bin/env python3
"""Inkwell's app icon, "Halo rim": the Glow orb on night, the plate's edge glowing indigo to coral.

Writes the SVG masters and renders every size the Mac and Windows builds need:
  design/icon/out/mac/icon_<n>x<n>[@2x].png  -> iconutil -> mac/AppIcon.icns
  design/icon/out/win/<n>.png                -> windows/Inkwell/Assets/Inkwell.ico
Rendering goes through a headless Chromium-family browser (--browser), so the blur and blend
match the design canvas. Small sizes (32 px and below) use simplified art: two solid discs, a
core and the rim, which is what stays readable at 16 px.

  python3 design/icon/make_icon.py --browser "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"
"""
import argparse, io, os, struct, subprocess, sys, tempfile
from pathlib import Path

INDIGO, CORAL, NIGHT = "#6B5CFF", "#FFA34D", "#121118"

def art(small: bool) -> str:
    """The icon in a 100 x 100 box: plate, rim glow, orb."""
    defs = ('<linearGradient id="rim" x1="0%" y1="0%" x2="100%" y2="100%">'
            f'<stop offset="0" stop-color="{INDIGO}"/><stop offset="1" stop-color="{CORAL}"/></linearGradient>'
            '<clipPath id="plate"><rect x="0" y="0" width="100" height="100" rx="22.5"/></clipPath>')
    plate = f'<rect x="0" y="0" width="100" height="100" rx="22.5" fill="{NIGHT}"/>'
    if small:
        return (f'<defs>{defs}</defs>{plate}'
                f'<circle cx="41" cy="51" r="18" fill="{INDIGO}"/>'
                f'<circle cx="59" cy="51" r="18" fill="{CORAL}" style="mix-blend-mode:screen"/>'
                '<circle cx="50" cy="51" r="6.5" fill="#fff" opacity=".9"/>'
                '<rect x="3" y="3" width="94" height="94" rx="20" fill="none" stroke="url(#rim)" stroke-width="6"/>')
    blur = lambda i, sd: f'<filter id="{i}" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="{sd}"/></filter>'
    s = 0.72
    return (f'<defs>{defs}{blur("b", 5)}{blur("c", 2.5)}{blur("r", 3)}</defs>{plate}'
            '<g clip-path="url(#plate)">'
            '<rect x="2" y="2" width="96" height="96" rx="21" fill="none" stroke="url(#rim)" stroke-width="7" filter="url(#r)"/>'
            f'<circle cx="{50-9*s}" cy="51" r="{21*s}" fill="{INDIGO}" filter="url(#b)"/>'
            f'<circle cx="{50+9*s}" cy="{51+1.5*s}" r="{19*s}" fill="{CORAL}" filter="url(#b)" style="mix-blend-mode:screen"/>'
            f'<circle cx="50" cy="51" r="{7*s}" fill="#fff" opacity=".85" filter="url(#c)"/>'
            '</g>'
            '<rect x="1.4" y="1.4" width="97.2" height="97.2" rx="21.3" fill="none" stroke="url(#rim)" stroke-width="1.6" opacity=".9"/>')

def mac_svg(px: int) -> str:
    # Apple's grid: the plate is 824 of 1024, inset 100, with a soft shadow under it.
    small = px <= 32
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="{px}" height="{px}">'
            '<defs><filter id="sh" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="14"/></filter></defs>'
            '<rect x="100" y="112" width="824" height="824" rx="185" fill="#000" opacity=".32" filter="url(#sh)"/>'
            f'<g transform="translate(100 100) scale(8.24)">{art(small)}</g></svg>')

def win_svg(px: int) -> str:
    # Windows draws icons as they are: the plate fills the square.
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" width="{px}" height="{px}">'
            f'{art(px <= 32)}</svg>')

def render(browser: str, svg: str, px: int, out: Path) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        page = Path(tmp) / "icon.html"
        page.write_text(f'<!doctype html><html><head><meta charset="utf-8"><style>html,body{{margin:0;background:transparent}}svg{{display:block}}</style></head><body>{svg}</body></html>')
        subprocess.run([browser, "--headless", "--disable-gpu", "--hide-scrollbars", "--force-device-scale-factor=1",
                        "--default-background-color=00000000", f"--window-size={px},{px}",
                        f"--screenshot={out}", page.as_uri()], check=True, capture_output=True, timeout=120)
    if not out.exists():
        sys.exit(f"render failed: {out}")

def write_ico(pngs: list[Path], out: Path) -> None:
    # An .ico of PNG entries (Windows Vista and later read them at every size).
    blobs = [p.read_bytes() for p in pngs]
    sizes = [struct.unpack(">II", b[16:24]) for b in blobs]
    head = struct.pack("<HHH", 0, 1, len(blobs))
    offset = 6 + 16 * len(blobs)
    entries = b""
    for (w, h), b in zip(sizes, blobs):
        entries += struct.pack("<BBBBHHII", w % 256, h % 256, 0, 0, 1, 32, len(b), offset)
        offset += len(b)
    out.write_bytes(head + entries + b"".join(blobs))

def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--browser", required=True)
    a = ap.parse_args()
    root = Path(__file__).resolve().parent
    out = root / "out"
    (root / "icon-mac.svg").write_text(mac_svg(1024))
    (root / "icon-win.svg").write_text(win_svg(256))
    iconset = out / "AppIcon.iconset"
    for n in (16, 32, 128, 256, 512):
        render(a.browser, mac_svg(n), n, iconset / f"icon_{n}x{n}.png")
        render(a.browser, mac_svg(2 * n), 2 * n, iconset / f"icon_{n}x{n}@2x.png")
    win = [16, 20, 24, 32, 40, 48, 64, 256]
    for n in win:
        render(a.browser, win_svg(n), n, out / "win" / f"{n}.png")
    repo = root.parent.parent
    subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(repo / "mac" / "AppIcon.icns")], check=True)
    write_ico([out / "win" / f"{n}.png" for n in win], repo / "windows" / "Inkwell" / "Assets" / "Inkwell.ico")
    print("wrote mac/AppIcon.icns and windows/Inkwell/Assets/Inkwell.ico")

if __name__ == "__main__":
    main()
