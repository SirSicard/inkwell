// Renders the social card (scripts/og/og.html) to public/og.png at 1200 × 630 with Playwright's
// Chromium, then recompresses it with ImageMagick to stay under ~300 KB (WhatsApp's soft cap).
//   node scripts/make-og.mjs
import { chromium } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const html = new URL('./og/og.html', import.meta.url).href;
const out = fileURLToPath(new URL('../public/og.png', import.meta.url));
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1200, height: 630 }, deviceScaleFactor: 1 });
await page.goto(html, { waitUntil: 'load' });
await page.evaluate(() => document.fonts.ready);
const fontOk = await page.evaluate(() => document.fonts.check('800 80px "Archivo Variable"'));
if (!fontOk) throw new Error('Archivo did not load; the card would render in a fallback face');
await page.screenshot({ path: out });
await browser.close();
// sRGB, no metadata, 8-bit, maximum lossless compression.
execFileSync('magick', [out, '-strip', '-colorspace', 'sRGB', '-depth', '8', '-define', 'png:compression-level=9', out]);
console.log(`wrote public/og.png, ${Math.round(statSync(out).size / 1024)} KB`);
