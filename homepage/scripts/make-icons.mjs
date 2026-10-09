// The favicon set, from the 1.0 app icon ("Halo rim": the Glow orb on night, the plate's edge glowing
// indigo to coral). The art is design/icon/make_icon.py's, in a 100 × 100 box: the full art with its
// blurs for 48 px and up, the simplified art (two solid discs, a core and the rim) that stays
// readable at 16 and 32 px. Rendered with Playwright's Chromium, so the blur and the screen blend
// match the app icon's own renders.
//   node scripts/make-icons.mjs
// Writes public/icon.svg, favicon.ico (16, 32, 48), apple-touch-icon.png (180, full bleed),
// icon-192.png, icon-512.png and icon-mask-512.png (full bleed, the orb inside the safe zone).
import { chromium } from '@playwright/test';
import { writeFileSync, readFileSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const PUBLIC = new URL('../public/', import.meta.url);
const INDIGO = '#6B5CFF', CORAL = '#FFA34D', NIGHT = '#121118';

const RIM = `<linearGradient id="rim" x1="0%" y1="0%" x2="100%" y2="100%"><stop offset="0" stop-color="${INDIGO}"/><stop offset="1" stop-color="${CORAL}"/></linearGradient>`;
const blur = (id, sd) => `<filter id="${id}" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="${sd}"/></filter>`;

/** The icon in a 100 × 100 box. `radius` 22.5 is the app's plate; 0 is full bleed (iOS and maskable
 *  icons are masked by the system). `orbScale` shrinks the orb about the centre (the maskable safe zone). */
function art({ small = false, radius = 22.5, rim = true, orbScale = 1 } = {}) {
  const plate = `<rect class="plate" x="0" y="0" width="100" height="100" rx="${radius}" fill="${NIGHT}"/>`;
  const clip = `<clipPath id="plate"><rect x="0" y="0" width="100" height="100" rx="${radius}"/></clipPath>`;
  const orbAt = (inner) => `<g transform="translate(50 51) scale(${orbScale}) translate(-50 -51)">${inner}</g>`;
  if (small) {
    return `<defs>${RIM}</defs>${plate}` +
      orbAt(`<circle cx="41" cy="51" r="18" fill="${INDIGO}"/>` +
        `<circle cx="59" cy="51" r="18" fill="${CORAL}" style="mix-blend-mode:screen"/>` +
        `<circle cx="50" cy="51" r="6.5" fill="#fff" opacity=".9"/>`) +
      (rim ? `<rect x="3" y="3" width="94" height="94" rx="${Math.max(0, radius - 2.5)}" fill="none" stroke="url(#rim)" stroke-width="6"/>` : '');
  }
  const s = 0.72;
  return `<defs>${RIM}${clip}${blur('b', 5)}${blur('c', 2.5)}${blur('r', 3)}</defs>${plate}` +
    `<g clip-path="url(#plate)">` +
    (rim ? `<rect x="2" y="2" width="96" height="96" rx="${Math.max(0, radius - 1.5)}" fill="none" stroke="url(#rim)" stroke-width="7" filter="url(#r)"/>` : '') +
    orbAt(`<circle cx="${50 - 9 * s}" cy="51" r="${21 * s}" fill="${INDIGO}" filter="url(#b)"/>` +
      `<circle cx="${50 + 9 * s}" cy="${51 + 1.5 * s}" r="${19 * s}" fill="${CORAL}" filter="url(#b)" style="mix-blend-mode:screen"/>` +
      `<circle cx="50" cy="51" r="${7 * s}" fill="#fff" opacity=".85" filter="url(#c)"/>`) +
    `</g>` +
    (rim ? `<rect x="1.4" y="1.4" width="97.2" height="97.2" rx="${Math.max(0, radius - 1.2)}" fill="none" stroke="url(#rim)" stroke-width="1.6" opacity=".9"/>` : '');
}

const svg = (inner, px) => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" width="${px}" height="${px}">${inner}</svg>`;

// The tab icon: the simplified art. In a dark browser the plate lifts a step, so its edge still
// reads against a near-black tab strip.
const favicon = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><style>@media (prefers-color-scheme: dark){.plate{fill:#1C1A24}}</style>${art({ small: true })}</svg>\n`;
writeFileSync(new URL('icon.svg', PUBLIC), favicon);

const tmp = mkdtempSync(join(tmpdir(), 'inkwell-icons-'));
const browser = await chromium.launch();
const page = await browser.newPage({ deviceScaleFactor: 1 });
async function png(inner, px, file) {
  await page.setViewportSize({ width: px, height: px });
  await page.setContent(`<!doctype html><html><body style="margin:0;background:transparent">${svg(inner, px)}</body></html>`);
  const out = join(tmp, file);
  await page.screenshot({ path: out, omitBackground: true, clip: { x: 0, y: 0, width: px, height: px } });
  return out;
}

const write = (src, name) => writeFileSync(new URL(name, PUBLIC), readFileSync(src));
write(await png(art(), 192, 'icon-192.png'), 'icon-192.png');
write(await png(art(), 512, 'icon-512.png'), 'icon-512.png');
write(await png(art({ radius: 0 }), 180, 'apple-touch-icon.png'), 'apple-touch-icon.png');
// Maskable: full bleed, no rim (the mask would cut it), the orb inside the central 80 %.
write(await png(art({ radius: 0, rim: false, orbScale: 0.85 }), 512, 'icon-mask-512.png'), 'icon-mask-512.png');

// favicon.ico: PNG entries (every browser since Vista-era Windows reads them).
const icoPngs = [await png(art({ small: true }), 16, 'f16.png'), await png(art({ small: true }), 32, 'f32.png'), await png(art(), 48, 'f48.png')];
const blobs = icoPngs.map((p) => readFileSync(p));
const head = Buffer.alloc(6);
head.writeUInt16LE(0, 0);
head.writeUInt16LE(1, 2);
head.writeUInt16LE(blobs.length, 4);
let offset = 6 + 16 * blobs.length;
const entries = blobs.map((b) => {
  const e = Buffer.alloc(16);
  const w = b.readUInt32BE(16), h = b.readUInt32BE(20);
  e.writeUInt8(w % 256, 0);
  e.writeUInt8(h % 256, 1);
  e.writeUInt16LE(1, 4);
  e.writeUInt16LE(32, 6);
  e.writeUInt32LE(b.length, 8);
  e.writeUInt32LE(offset, 12);
  offset += b.length;
  return e;
});
writeFileSync(new URL('favicon.ico', PUBLIC), Buffer.concat([head, ...entries, ...blobs]));

await browser.close();
rmSync(tmp, { recursive: true, force: true });
console.log('wrote public/icon.svg, favicon.ico, apple-touch-icon.png, icon-192.png, icon-512.png, icon-mask-512.png');
