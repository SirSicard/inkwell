// Renders the 1.0 social card (scripts/og/og-1.0.html) to public/og-1.0.png at 1200 × 630 with
// Playwright's Chromium, then recompresses it with ImageMagick to stay under ~300 KB (WhatsApp's
// soft cap). A new file name for 1.0, so link previews do not keep the cached 0.2 card.
//   node scripts/make-og.mjs
// The card imports the page's orb module (TypeScript), so this serves homepage/ on a loopback port
// and strips the types on the way (node:module's stripTypeScriptTypes).
import { chromium } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { readFileSync, statSync, existsSync } from 'node:fs';
import { createServer } from 'node:http';
import { stripTypeScriptTypes } from 'node:module';
import { extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const out = fileURLToPath(new URL('../public/og-1.0.png', import.meta.url));
const TYPES = { '.html': 'text/html', '.woff2': 'font/woff2', '.js': 'text/javascript' };

const server = createServer((req, res) => {
  let path = normalize(join(root, decodeURIComponent(new URL(req.url, 'http://x').pathname)));
  if (!path.startsWith(root)) return res.writeHead(403).end();
  if (!extname(path) && existsSync(`${path}.ts`)) path += '.ts'; // the module's extension-less imports
  if (!existsSync(path)) return res.writeHead(404).end();
  if (path.endsWith('.ts')) {
    const js = stripTypeScriptTypes(readFileSync(path, 'utf8'));
    return res.writeHead(200, { 'content-type': 'text/javascript' }).end(js);
  }
  res.writeHead(200, { 'content-type': TYPES[extname(path)] ?? 'application/octet-stream' }).end(readFileSync(path));
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const { port } = server.address();

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1200, height: 630 }, deviceScaleFactor: 1 });
const errors = [];
page.on('pageerror', (e) => errors.push(String(e)));
await page.goto(`http://127.0.0.1:${port}/scripts/og/og-1.0.html`, { waitUntil: 'load' });
await page.waitForSelector('body[data-ready]', { timeout: 10000 }).catch(() => {});
if (errors.length) throw new Error(`the card failed: ${errors.join('; ')}`);
const fontOk = await page.evaluate(() => document.fonts.check('500 68px "Newsreader Variable"'));
if (!fontOk) throw new Error('Newsreader did not load; the card would render in a fallback face');
await page.screenshot({ path: out });
await browser.close();
server.close();
// sRGB, no metadata, 256 colours without dithering, maximum compression: the orb's grain makes a
// truecolour PNG about 370 KB, and a dither adds more noise than it hides; this lands near 90 KB.
execFileSync('magick', [out, '-strip', '-colorspace', 'sRGB', '+dither', '-colors', '256', '-define', 'png:compression-level=9', out]);
console.log(`wrote public/og-1.0.png, ${Math.round(statSync(out).size / 1024)} KB`);
