# Landing Page - Research

## Stack: Astro 5 + React Islands
- 90% static marketing content + one heavy WebGL hero island
- Astro ships zero JS by default, hydrates only interactive parts
- Better than Next.js (too much JS for a marketing page) or Vite SPA (kills SEO)

```
Astro 5 + React 19 islands + Tailwind 4 + R3F + GSAP ScrollTrigger + Vercel
```

## Hero Section
Hybrid pattern (linear.app / warp.dev style):
- Full-viewport ink blob shader as WebGL background (R3F)
- App window mockup floating on top
- Headline: "Your voice, in ink." cream text on dark
- Mouse-reactive ink ripple
- Platform-detected download button

## Ink Shader on Web
Ray-marched metaballs in fragment shader via R3F:
- Smooth-min SDF blending for organic blob merging
- Simplex noise displacement for ink texture
- Dark bg (#0A0A0A), ink blobs in blue-black, cream Fresnel highlights
- Mouse position as uniform for interaction

References: Codrops metaballs tutorial, Jamie Wong metaballs + WebGL, Shadertoy

## Scroll Animations: GSAP ScrollTrigger
- Used by 80%+ of awwwards winners
- Scrub, pin sections, timeline sequencing all built-in
- Only animate `transform` and `opacity` (GPU-composited)
- Works in Astro via `client:load` script or React island

## Download Button
- `navigator.userAgentData?.platform` for detection
- Primary: "Download for {platform}" with icon
- Secondary: "Also available for [others]" as links
- Links to GitHub Releases

## Page Sections (top to bottom)
1. **Nav**: fixed, transparent → solid on scroll
2. **Hero**: ink shader bg + app mockup + headline + CTA
3. **Logos**: "Built with Tauri, Rust, Whisper"
4. **Features**: 3-4 cards (local privacy, real-time, offline, custom vocab)
5. **How it works**: 3-step animated (speak → process → text)
6. **Demo**: embedded video or animated GIF
7. **Comparison**: table vs Otter.ai, WhisperFlow
8. **Testimonials**: 3 dark cards
9. **Pricing**: Free / Pro or "Free forever"
10. **Final CTA**: full-bleed ink shader, download buttons
11. **Footer**: links, GitHub, socials

## Performance
- Shader only in viewport (`client:visible`, pause when scrolled past)
- Render shader at 0.5x-0.75x DPR (imperceptible on organic shapes)
- Lazy load everything below fold
- Target: LCP <2.5s, total JS <150KB (excluding lazy R3F)

## Reference Sites
- linear.app (dark theme, gradient meshes, scroll animations)
- raycast.com (download UX, app screenshot hero)
- warp.dev (terminal app marketing, WebGL bg)
- cursor.com (AI tool positioning, comparison)
- Codrops metaballs tutorial (exact shader technique)

## Deploy
- Astro static build → Vercel or Cloudflare Pages
- GitHub Actions CI
- Domain: inkwell.app or similar
