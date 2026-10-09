// Writes the screenshot PLACEHOLDERS listed in src/data/shots.ts to public/shots/: a flat Glow card
// per shot and theme at the size the real capture is served at, labelled with the shot's id, so the
// layout is final before the captures exist. Skips any shot that is no longer a placeholder (the
// real ones come from scripts/encode-shots.mjs).
//   node scripts/make-shots.mjs
// Needs Playwright's Chromium and ImageMagick (`magick`) for the WebP encode.
import { chromium } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { SHOTS } from '../src/data/shots.ts';

const out = fileURLToPath(new URL('../public/shots/', import.meta.url));
mkdirSync(out, { recursive: true });
const tmp = mkdtempSync(join(tmpdir(), 'inkwell-shots-'));

const THEME = {
  light: { page: '#EDE7DF', card: '#FBF8F4', text: '#1D1B2E', muted: '#6A6577', rule: '#E2DACE' },
  dark: { page: '#0B0A0F', card: '#121118', text: '#EDEAF2', muted: '#A49FB4', rule: '#2A2833' },
};

const browser = await chromium.launch();
const page = await browser.newPage({ deviceScaleFactor: 1 });
for (const shot of Object.values(SHOTS)) {
  if (!shot.placeholder) continue;
  const themes = shot.themes === 'both' ? ['light', 'dark'] : [shot.themes];
  for (const theme of themes) {
    const t = THEME[theme];
    const name = shot.themes === 'both' ? `${shot.file}-${theme}` : shot.themes === 'dark' ? `${shot.file}-dark` : shot.file;
    await page.setViewportSize({ width: shot.width, height: shot.height });
    await page.setContent(`<!doctype html><html><body style="margin:0;width:${shot.width}px;height:${shot.height}px;background:${t.card};
      font-family:system-ui,-apple-system,sans-serif;display:grid;place-items:center;color:${t.muted}">
      <div style="position:absolute;inset:${Math.round(shot.height * 0.06)}px;border:2px dashed ${t.rule};border-radius:22px"></div>
      <div style="text-align:center;max-width:70%">
        <div style="font:500 ${Math.round(shot.height * 0.09)}px Georgia,serif;color:${t.text}">${shot.id}</div>
        <div style="margin-top:12px;font-size:${Math.max(18, Math.round(shot.height * 0.028))}px;line-height:1.4">${shot.alt}</div>
        <div style="margin-top:10px;font-size:${Math.max(15, Math.round(shot.height * 0.02))}px">Placeholder, ${shot.width} × ${shot.height}, ${theme}</div>
      </div></body></html>`);
    const png = join(tmp, `${name}.png`);
    await page.screenshot({ path: png });
    execFileSync('magick', [png, '-strip', '-quality', '82', join(out, `${name}.webp`)]);
    console.log(`wrote public/shots/${name}.webp`);
  }
}
await browser.close();
rmSync(tmp, { recursive: true, force: true });
