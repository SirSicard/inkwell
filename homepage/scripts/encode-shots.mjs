// Encodes the real screenshots listed in src/data/shots.ts from the raw Mac captures into
// public/shots/: converted to sRGB, cropped where a shot needs it, and written as WebP (quality 82)
// at two widths, the 1x (half the shot's width) and the 2x (its full width). Alpha is kept, so the
// window shots keep their rounded corners.
//   node scripts/encode-shots.mjs <dir with the raw PNGs>
// Needs ImageMagick 7 (`magick`) with lcms. Raw names are the shot's file stem plus the theme, e.g.
// l1-library-light.png. Placeholders are left to scripts/make-shots.mjs.
import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { SHOTS } from '../src/data/shots.ts';

const raw = process.argv[2];
if (!raw || !existsSync(raw)) {
  console.error('usage: node scripts/encode-shots.mjs <dir with the raw PNGs>');
  process.exit(1);
}
const out = fileURLToPath(new URL('../public/shots/', import.meta.url));

// The Mac's captures carry the display's profile; macOS ships sRGB, so convert to it when present.
const SRGB = '/System/Library/ColorSync/Profiles/sRGB Profile.icc';
const toSrgb = existsSync(SRGB) ? ['-profile', SRGB] : [];

// Per-shot steps before the resize, in raw (2x) pixels.
const PREP = {
  // The 2880 × 1800 region capture, cut to the alert with some of the dimmed window around it.
  'p3-consent': ['-crop', '1040x640+920+570', '+repage'],
  // The Drops are captured over the wallpaper: keep the pill (its outer hairline included) and drop
  // the wallpaper, so the pill sits on the page's own ground.
  'd1-drop-meeting': pill(),
  'd1-drop-final': pill(),
};
function pill() {
  return [
    '(', '-size', '960x220', 'xc:black', '-fill', 'white', '-draw', 'roundrectangle 35,37 916,190 77,77', '-alpha', 'off', ')',
    '-compose', 'CopyOpacity', '-composite', '-crop', '890x162+31+33', '+repage',
  ];
}

for (const shot of Object.values(SHOTS)) {
  if (shot.placeholder) continue;
  const themes = shot.themes === 'both' ? ['light', 'dark'] : [shot.themes];
  for (const theme of themes) {
    const src = join(raw, `${shot.file}-${theme}.png`);
    const stem = shot.themes === 'light' ? shot.file : `${shot.file}-${theme}`;
    for (const [scale, w, h] of [['1x', shot.width / 2, shot.height / 2], ['2x', shot.width, shot.height]]) {
      const dest = join(out, `${stem}-${scale}.webp`);
      execFileSync('magick', [
        '-quiet', src, ...toSrgb, ...(PREP[shot.file] ?? []),
        '-filter', 'Lanczos', '-resize', `${w}x${h}!`, '-strip',
        '-quality', '82', '-define', 'webp:method=6', '-define', 'webp:alpha-quality=100', dest,
      ]);
      console.log(`wrote public/shots/${stem}-${scale}.webp (${w} × ${h})`);
    }
  }
}
