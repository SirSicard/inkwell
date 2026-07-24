# Inkwell homepage

The marketing site for [Inkwell](https://github.com/SirSicard/inkwell), the local-first desktop dictation app. It lives in the app's own repository, one directory down, so the copy and the code can never drift apart unnoticed.

Next.js 16 (App Router) · React 19 · Tailwind CSS v4 · Geist Sans/Mono via `next/font`.

## Run it

```bash
cd homepage
npm install
npm run dev      # http://localhost:3000
```

Other scripts: `npm run build` (production build), `npm start` (serve the build).

Node 20+ is expected — the app's CI uses it.

## Where things live

```
app/
  layout.tsx        metadata, fonts, the pre-paint `.js` flag for scroll reveals
  page.tsx          section order, the only place the page is composed
  globals.css       design tokens, surfaces, buttons, focus and motion rules
components/
  InkCanvas.tsx     the WebGL ink shader, ported from the app
  Hero.tsx          wordmark, one-liner, primary CTA, ink panel
  HowItWorks.tsx    the four-step dictation loop
  FeaturesSection.tsx
  ModelsSection.tsx a curated slice of the in-app model catalogue
  PrivacySection.tsx what stays local, and the three network calls
  DownloadSection.tsx platform-detected CTA + the unsigned-build note
  SupportSection.tsx the voluntary donation
  SiteHeader.tsx / SiteFooter.tsx
  Reveal.tsx        scroll-reveal wrapper (no-op under reduced motion)
  icons.tsx         inline SVGs
lib/
  constants.ts      every outbound URL and support detail
  platform.ts       OS detection for the download CTA
```

## Constants you must edit before launch

All of them are in **`lib/constants.ts`**. Nothing is hardcoded a second time anywhere else — change it there and the whole page follows.

| Constant | Status | What to do |
| --- | --- | --- |
| `DONATION_URL` | **placeholder** (`buymeacoffee.com/REPLACE_ME`) | Replace with the real handle. Until then the donate button 404s. |
| `SITE_URL` | **placeholder** (`https://inkwell.example`) | Set the real domain; it drives the canonical URL and the absolute OG image URL. |
| `APP_VERSION` | `0.1.1` | Keep in step with `version` in `../src-tauri/tauri.conf.json`. |
| `GITHUB_URL` | live | Repo, and the base for the releases/issues/licence links. |
| `DONATION_SUGGESTED` | `€10` | Suggested tip shown in the support section. |

## Assets still missing

- `public/og.png` — 1200×630 social card, referenced by `app/layout.tsx`. Suggested: charcoal field, cream ink blob, the word INKWELL. Until it exists, link previews fall back to no image.
- Product screenshots. There is no screenshot section yet because there are no exportable shots of the app; when they exist, add them between "How it works" and "Features".

## Copy rules

Everything factual on this page is checked against the app source in `../src` and `../src-tauri` — model names and sizes against `src/tabs/ModelsTab.tsx`, the pipeline against `src-tauri/src/pipeline.rs`, formats against `src-tauri/src/filetranscribe.rs` and `src-tauri/src/export.rs`, build targets against `.github/workflows/build.yml`. If you cannot point at a file, the claim does not go on the page.

Three things are deliberately stated rather than hidden: the builds are unsigned, per-app style overrides are Windows-only, and there is no live-as-you-speak transcription.

## Design notes

- **Solid surfaces, not glass.** The app UI uses opaque panels; the site now matches. Stacked `backdrop-filter` over a live WebGL canvas costs GPU and hurts text contrast, so depth comes from elevation and hairline borders instead.
- **Tokens mirror the app.** `#0e0e11` base, `#18181d` surface, `#f0ede8` text, `#c8956c` copper accent — the same values as `../src/index.css`.
- **The ink is the brand.** `InkCanvas` runs the app's simplex-noise shader with the audio uniforms pinned at zero. It pauses off-screen and when the tab is hidden, and under `prefers-reduced-motion` it paints exactly one frame. Without WebGL a static gradient stands in.

## Deploy

Vercel is the path of least resistance: import the repo, set the **root directory** to `homepage`, and take the framework defaults (`next build`). Set the production domain to whatever `SITE_URL` says.

Anything that runs Node works too — `npm run build && npm start` behind a reverse proxy. The site is fully static apart from React hydration, so `output: "export"` in `next.config.ts` would also work if a plain static host is preferred.

## Licence

MIT, same as the app.
