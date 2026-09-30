# Inkwell homepage

The marketing site for [Inkwell](https://github.com/SirSicard/inkwell), the local-first desktop dictation app. It lives in the app's own repository, one directory down, so the copy and the code can never drift apart unnoticed.

A static Astro 7 site: one page and a designed 404, about 6 KB of gzipped JavaScript, and every word readable with JavaScript off. Tailwind CSS v4 · Archivo, self-hosted through Fontsource.

## Run it

```bash
cd homepage
npm ci
npm run dev      # http://localhost:4321
```

`npm run build` writes the static site to `dist/`; `npm run preview` serves that build. Node 22.12 or newer (Astro 7's floor); Vercel builds with Node 24.

## Where things live

```
src/pages/index.astro     section order, metadata and JSON-LD
src/pages/404.astro       the designed 404
src/layouts/Base.astro    <head>, fonts, the pre-paint `.js` flag
src/components/           one file per section: Hero, Features, Models, Privacy,
                          Install, Cost, CodeSigning, Colophon, SiteHeader
src/lib/constants.ts      MAC_VERSION and WINDOWS_VERSION, every outbound URL, the models
src/lib/release.ts        download links, checked against src/data/release.json at build time
src/lib/ink-field.ts      the <ink-field> element: the app's ink shader (webgl-noise notice inside)
src/lib/hotkey-demo.ts    the hero's hold-a-key demo
src/lib/platform.ts       points the download button at the visitor's OS
src/styles/base.css       design tokens and layout
scripts/                  make-icons.mjs and make-og.mjs (favicons, social card),
                          snapshot-release.mjs (see below)
```

## After a release

The site only advertises a version that has a release ([RELEASING.md](../docs/RELEASING.md)). Each platform names its own version, so the Mac and Windows can move to a release one at a time. Once the release exists:

1. Set `MAC_VERSION`, `WINDOWS_VERSION` or both in `src/lib/constants.ts`.
2. Run `node scripts/snapshot-release.mjs` (needs `gh`). It copies the real asset names and sizes of each release those versions name into `src/data/release.json`.
3. Push. The build fails rather than ship a download link to a file its release doesn't have.

## What the page keeps to

- Everything meant to be read is in the HTML. JavaScript only enhances: the download button's OS pick, the demo, the ink.
- The ink canvas never blocks the page. A poster shows until its first frame; it pauses off-screen and stays still under reduced motion.
- No cookies, no analytics, no third-party requests.
