// Generates the favicon set from the app's own ink: the ink shader's field (the same simplex noise,
// warp and drops as src/lib/ink-field.ts), evaluated once on a grid, traced at its edge into a vector
// blot, then rasterised with ImageMagick (`magick`).
//   node scripts/make-icons.mjs            writes public/ icons + manifest, and src/data/blot.json
//   node scripts/make-icons.mjs --preview  also writes a contact sheet of candidate frames to /tmp
// Needs ImageMagick 7 on PATH. Build-time only; nothing here ships as JavaScript.
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const PUBLIC = new URL('../public/', import.meta.url);
const DATA = new URL('../src/data/', import.meta.url);
const INK = '#0f0f0f';
const PAPER = '#f0ede8';

// The frame: shader time and look. Chosen from the preview sheet for a blot that reads at 16 px.
const FRAME = { time: 3.2, size: 0.3, warp: 0.14, sat: 0.9, spread: 0.9 }; // t = 3.2 is also the page's still frame

/* ── The shader's field, in JS ──────────────────────────────────────────────────────────────────
   snoise() is 2D simplex noise from webgl-noise (Ian McEwan, Ashima Arts; MIT, notice in
   src/lib/ink-field.ts), transcribed from the GLSL. */
const mod289 = (x) => x - Math.floor(x / 289) * 289;
const permute = (x) => mod289((x * 34 + 1) * x);
const fract = (x) => x - Math.floor(x);
function snoise(vx, vy) {
  const C0 = 0.211324865405187, C1 = 0.366025403784439, C2 = -0.577350269189626, C3 = 0.024390243902439;
  let ix = Math.floor(vx + (vx + vy) * C1);
  let iy = Math.floor(vy + (vx + vy) * C1);
  const x0x = vx - ix + (ix + iy) * C0;
  const x0y = vy - iy + (ix + iy) * C0;
  const i1x = x0x > x0y ? 1 : 0;
  const i1y = x0x > x0y ? 0 : 1;
  const x12 = [x0x + C0 - i1x, x0y + C0 - i1y, x0x + C2, x0y + C2];
  ix = mod289(ix);
  iy = mod289(iy);
  const p = [0, i1y, 1].map((d, n) => permute(permute(iy + d) + ix + [0, i1x, 1][n]));
  const d0 = x0x * x0x + x0y * x0y;
  const d1 = x12[0] * x12[0] + x12[1] * x12[1];
  const d2 = x12[2] * x12[2] + x12[3] * x12[3];
  let m = [d0, d1, d2].map((d) => Math.max(0.5 - d, 0));
  m = m.map((v) => v * v * v * v);
  const x = p.map((v) => 2 * fract(v * C3) - 1);
  const h = x.map((v) => Math.abs(v) - 0.5);
  const a0 = x.map((v) => v - Math.floor(v + 0.5));
  m = m.map((v, n) => v * (1.79284291400159 - 0.85373472095314 * (a0[n] * a0[n] + h[n] * h[n])));
  const g0 = a0[0] * x0x + h[0] * x0y;
  const g1 = a0[1] * x12[0] + h[1] * x12[1];
  const g2 = a0[2] * x12[2] + h[2] * x12[3];
  return 130 * (m[0] * g0 + m[1] * g1 + m[2] * g2);
}
const fbm = (x, y) => snoise(x, y) * 0.68 + snoise(x * 2 + 5.2, y * 2 + 5.2) * 0.32;
const drop = (px, py, cx, cy, r) => (r * r) / Math.max((px - cx) ** 2 + (py - cy) ** 2, 1e-5);

/** The shader's field at a point of the unit square (y up, as gl_FragCoord), aspect 1, amp 0. */
function field(x, y, o) {
  const t = o.time * 0.17325;
  const wx = fbm(x * 1.75 + t * 0.17, y * 1.75 + t * 0.1);
  const wy = fbm(x * 1.75 - t * 0.12 + 19.7, y * 1.75 + t * 0.15 + 19.7);
  const px = x + wx * o.warp;
  const py = y + wy * o.warp;
  const r = o.size, k = o.spread, q = r * o.sat;
  return (
    drop(px, py, 0.5 + 0.05 * Math.sin(t * 0.9), 0.5 + 0.06 + 0.05 * Math.cos(t * 0.7), r) +
    drop(px, py, 0.5 + k * (-0.26 + 0.09 * Math.sin(t * 1.1 + 1)), 0.5 + k * (-0.3 + 0.08 * Math.cos(t * 0.95)), q * 0.53) +
    drop(px, py, 0.5 + k * (0.3 + 0.08 * Math.cos(t * 0.85 + 2)), 0.5 + k * (0.34 + 0.09 * Math.sin(t * 1.05 + 0.7)), q * 0.46) +
    drop(px, py, 0.5 + k * (0.22 + 0.07 * Math.sin(t * 1.2 + 3.1)), 0.5 + k * (-0.58 + 0.08 * Math.cos(t * 1.0 + 1.7)), q * 0.35)
  );
}

/* ── Trace the edge (f = 1) with marching squares, over a margin around the frame ─────────────── */
function trace(o, n = 360, lo = -0.35, hi = 1.35) {
  const step = (hi - lo) / n;
  const v = [];
  for (let j = 0; j <= n; j++) {
    v.push([]);
    for (let i = 0; i <= n; i++) v[j].push(field(lo + i * step, lo + j * step, o) - 1);
  }
  // Segments per cell, with edge points interpolated; then stitched into closed loops.
  const key = (p) => `${p[0].toFixed(5)},${p[1].toFixed(5)}`;
  const lerp = (a, b, va, vb) => { const t = va / (va - vb); return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]; };
  const segs = [];
  for (let j = 0; j < n; j++) {
    for (let i = 0; i < n; i++) {
      const P = [[i, j], [i + 1, j], [i + 1, j + 1], [i, j + 1]];
      const V = P.map(([a, b]) => v[b][a]);
      const pts = [];
      for (let e = 0; e < 4; e++) {
        const a = e, b = (e + 1) % 4;
        if ((V[a] > 0) !== (V[b] > 0)) pts.push(lerp(P[a], P[b], V[a], V[b]));
      }
      if (pts.length === 2) segs.push(pts);
      else if (pts.length === 4) {
        // Saddle: all four edges crossed (pts are edges 0-3 in order). The cell centre decides the
        // topology: when it shares corner 0's side, corners 1 and 3 are cut off; otherwise 0 and 2 are.
        const centre = (V[0] + V[1] + V[2] + V[3]) / 4;
        if (centre > 0 === V[0] > 0) { segs.push([pts[0], pts[1]]); segs.push([pts[2], pts[3]]); }
        else { segs.push([pts[3], pts[0]]); segs.push([pts[1], pts[2]]); }
      }
    }
  }
  const adj = new Map();
  for (const [a, b] of segs) {
    for (const [p, q] of [[a, b], [b, a]]) {
      const k = key(p);
      if (!adj.has(k)) adj.set(k, { p, next: [] });
      adj.get(k).next.push(q);
    }
  }
  const seen = new Set();
  const loops = [];
  for (const [k0, node] of adj) {
    if (seen.has(k0)) continue;
    const loop = [node.p];
    seen.add(k0);
    let cur = node;
    for (;;) {
      const nxt = cur.next.find((q) => !seen.has(key(q)));
      if (!nxt) break;
      seen.add(key(nxt));
      loop.push(nxt);
      cur = adj.get(key(nxt));
    }
    if (loop.length > 12) loops.push(loop.map(([gx, gy]) => [lo + gx * step, lo + gy * step]));
  }
  return loops;
}

const area = (l) => Math.abs(l.reduce((s, p, i) => { const q = l[(i + 1) % l.length]; return s + p[0] * q[1] - q[0] * p[1]; }, 0) / 2);

/** Ramer-Douglas-Peucker on a closed loop, then a Catmull-Rom spline as cubic Béziers. */
function simplify(pts, eps) {
  if (pts.length < 3) return pts;
  const [a, b] = [pts[0], pts[pts.length - 1]];
  let dmax = 0, idx = 0;
  for (let i = 1; i < pts.length - 1; i++) {
    const p = pts[i];
    const d = Math.abs((b[1] - a[1]) * p[0] - (b[0] - a[0]) * p[1] + b[0] * a[1] - b[1] * a[0]) / Math.hypot(b[0] - a[0], b[1] - a[1]);
    if (d > dmax) { dmax = d; idx = i; }
  }
  if (dmax <= eps) return [a, b];
  return [...simplify(pts.slice(0, idx + 1), eps).slice(0, -1), ...simplify(pts.slice(idx), eps)];
}
function smoothPath(loop, fmt) {
  const n = loop.length;
  let d = `M${fmt(loop[0])}`;
  for (let i = 0; i < n; i++) {
    const p0 = loop[(i - 1 + n) % n], p1 = loop[i], p2 = loop[(i + 1) % n], p3 = loop[(i + 2) % n];
    const c1 = [p1[0] + (p2[0] - p0[0]) / 6, p1[1] + (p2[1] - p0[1]) / 6];
    const c2 = [p2[0] - (p3[0] - p1[0]) / 6, p2[1] - (p3[1] - p1[1]) / 6];
    d += `C${fmt(c1)} ${fmt(c2)} ${fmt(p2)}`;
  }
  return d + 'Z';
}

/** The blot as an SVG path in a 0..size box, centred with the given padding, y flipped to SVG. */
function blotPath(o, size = 64, pad = 3) {
  const loops = trace(o).filter((l) => area(l) > 0.0015).sort((a, b) => area(b) - area(a)).slice(0, 3);
  const all = loops.flat();
  const [x0, x1] = [Math.min(...all.map((p) => p[0])), Math.max(...all.map((p) => p[0]))];
  const [y0, y1] = [Math.min(...all.map((p) => p[1])), Math.max(...all.map((p) => p[1]))];
  const s = (size - 2 * pad) / Math.max(x1 - x0, y1 - y0);
  const ox = (size - (x1 - x0) * s) / 2, oy = (size - (y1 - y0) * s) / 2;
  const map = ([x, y]) => [ox + (x - x0) * s, oy + (y1 - y) * s];
  const fmt = (p) => `${p[0].toFixed(2)} ${p[1].toFixed(2)}`;
  return loops.map((l) => {
    // RDP needs distinct ends: split the closed loop at its point farthest from the start.
    let far = 0;
    l.forEach((p, i) => { if (Math.hypot(p[0] - l[0][0], p[1] - l[0][1]) > Math.hypot(l[far][0] - l[0][0], l[far][1] - l[0][1])) far = i; });
    const half1 = simplify(l.slice(0, far + 1), 0.0035);
    const half2 = simplify([...l.slice(far), l[0]], 0.0035);
    const pts = [...half1.slice(0, -1), ...half2.slice(0, -1)].map(map);
    return smoothPath(pts, fmt);
  }).join('');
}

const magick = (...args) => execFileSync('magick', args, { stdio: 'inherit' });

if (process.argv.includes('--preview')) {
  const dir = mkdtempSync(join(tmpdir(), 'ink-icons-'));
  const files = [];
  const size = Number(process.argv[process.argv.indexOf('--preview') + 1]) || FRAME.size;
  const warp = Number(process.argv[process.argv.indexOf('--preview') + 2]) || FRAME.warp;
  for (const time of [3.2, 11, 17, 25.9, 33, 41, 52, 60, 77]) {
    const d = blotPath({ ...FRAME, size, warp, time }, 64, 3);
    const f = join(dir, `t${time}.svg`);
    writeFileSync(f, `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" width="256" height="256"><rect width="64" height="64" fill="${PAPER}"/><path d="${d}" fill="${INK}"/></svg>`);
    files.push(f);
  }
  magick(...files, '-bordercolor', '#ffffff', '-border', '6', '+append', join(dir, 'sheet.png')); // left to right in the order above
  console.log(join(dir, 'sheet.png'));
  process.exit(0);
}

const d64 = blotPath(FRAME, 64, 3);
writeFileSync(new URL('blot.json', DATA), JSON.stringify({ viewBox: '0 0 64 64', d: d64, frame: FRAME }, null, 2) + '\n');

// icon.svg: the ink on a transparent ground; in a dark browser UI it turns to paper so it stays visible.
writeFileSync(
  new URL('icon.svg', PUBLIC),
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><style>path{fill:${INK}}@media (prefers-color-scheme:dark){path{fill:${PAPER}}}</style><path d="${d64}"/></svg>\n`,
);

const tmp = mkdtempSync(join(tmpdir(), 'ink-icons-'));
const svgOn = (bg, pad, name) => {
  const d = blotPath(FRAME, 64, pad);
  const f = join(tmp, name);
  writeFileSync(f, `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" width="1024" height="1024">${bg ? `<rect width="64" height="64" fill="${bg}"/>` : ''}<path d="${d}" fill="${INK}"/></svg>`);
  return f;
};
const out = (name) => new URL(name, PUBLIC).pathname;

// favicon.ico: 16, 32 and 48, the ink on transparent (tabs in both themes show a light pill or not;
// the .ico is the legacy fallback, icon.svg carries the dark-mode variant).
const bare = svgOn(null, 3, 'bare.svg');
magick('-background', 'none', '-density', '96', bare, '-define', 'icon:auto-resize=48,32,16', out('favicon.ico'));
// Opaque icons on paper for home screens and the manifest.
const onPaper = svgOn(PAPER, 10, 'paper.svg');
magick('-background', 'none', '-density', '96', onPaper, '-resize', '180x180', '-strip', '-depth', '8', out('apple-touch-icon.png'));
magick('-background', 'none', '-density', '96', onPaper, '-resize', '192x192', '-strip', '-depth', '8', out('icon-192.png'));
magick('-background', 'none', '-density', '96', onPaper, '-resize', '512x512', '-strip', '-depth', '8', out('icon-512.png'));
// Maskable: the blot inside the 80% safe circle.
const mask = svgOn(PAPER, 16, 'mask.svg');
magick('-background', 'none', '-density', '96', mask, '-resize', '512x512', '-strip', '-depth', '8', out('icon-mask-512.png'));

writeFileSync(
  new URL('manifest.webmanifest', PUBLIC),
  JSON.stringify(
    {
      name: 'Inkwell',
      short_name: 'Inkwell',
      description: 'Local-first dictation for the desktop. Free, MIT licensed, no account.',
      start_url: '/',
      display: 'browser',
      background_color: PAPER,
      theme_color: PAPER,
      icons: [
        { src: '/icon-192.png', sizes: '192x192', type: 'image/png' },
        { src: '/icon-512.png', sizes: '512x512', type: 'image/png' },
        { src: '/icon-mask-512.png', sizes: '512x512', type: 'image/png', purpose: 'maskable' },
      ],
    },
    null,
    2,
  ) + '\n',
);
console.log('wrote public/icon.svg, favicon.ico, apple-touch-icon.png, icon-192.png, icon-512.png, icon-mask-512.png, manifest.webmanifest; src/data/blot.json');
