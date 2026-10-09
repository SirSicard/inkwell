# Inkwell homepage

The marketing site for [Inkwell](https://github.com/SirSicard/inkwell), local dictation and meeting notes for the Mac and Windows. It lives in the app's own repository, one directory down, so the copy and the code can never drift apart unnoticed.

A static Astro 7 site: one page and a designed 404, every word readable with JavaScript off. Tailwind CSS v4 · Glow's tokens (`design/tokens.json`) · the system face for text and Newsreader (OFL-1.1, self-hosted through Fontsource) for headlines.

## Run it

```bash
cd homepage
npm ci
npm run dev      # http://localhost:4321
npm test         # the download links and the token copy (Node 23.6 or newer)
```

`npm run build` writes the static site to `dist/`; `npm run preview` serves that build. Node 22.12 or newer (Astro 7's floor); Vercel builds with Node 24.

## Where things live

```
src/pages/index.astro     section order, metadata and JSON-LD
src/pages/404.astro       the designed 404
src/layouts/Base.astro    <head>, the font, the pre-paint `js` flag and theme
src/components/           one file per section: Hero, Dictation, Meetings, Library (with Stats),
                          Models, Privacy, Consent, Install, CodeSigning, Colophon; plus
                          SiteHeader, Downloads and Shot
src/data/glow.ts          the page's copy of Glow's tokens (tests/glow.test.mjs keeps it in step)
src/data/shots.ts         the screenshots, with the list (W1 is still a placeholder)
src/lib/constants.ts      APP_VERSION, every outbound URL, the model list
src/lib/release.ts        download links, checked against src/data/release.json at build time
src/lib/release-assets.ts the rules for which release files the page links to (tested)
src/lib/glow-orb.ts       the app's orb shader (shaders/ink.wgsl) ported to WebGL, with its motion
src/lib/hero.ts           the hero: the wandering orb, the Drop's orb and the coded dictation
src/lib/platform.ts       leads with the visitor's OS; notes for Intel Macs, Linux, Windows on ARM
src/lib/theme.ts          the light / system / dark switch, and copy buttons
src/styles/base.css       tokens and layout
scripts/                  make-icons.mjs (favicons from the 1.0 app icon), make-og.mjs (the
                          social card, public/og-1.0.png), encode-shots.mjs (the screenshots'
                          WebPs from the raw captures), make-shots.mjs (the placeholders),
                          snapshot-release.mjs (see below)
tests/                    node --test
```

## After a release

The site only advertises a version that has a release ([RELEASING.md](../docs/RELEASING.md), step 8). Once the release exists:

1. Set `APP_VERSION` in `src/lib/constants.ts`.
2. Run `node scripts/snapshot-release.mjs` (needs `gh`). It copies the release's real asset names, sizes and SHA-256 digests into `src/data/release.json`.
3. Push. The build fails rather than ship a download link to a file the release doesn't have: from 1.0 that is the Mac dmg, the Windows installer and the Windows sums file.

## What the page keeps to

- Everything meant to be read is in the HTML. JavaScript only enhances: the OS pick, the theme switch, the orb and the demo.
- The orb never blocks the page: it starts once the page is idle, a CSS glow stands in until then (and without WebGL), it draws nothing while it holds still, it pauses off screen and in a hidden tab, and reduced motion gets one still frame.
- No cookies, no analytics, no third-party requests. The theme choice is kept in the visitor's own browser.
