// The Inkwell homepage. Static output, zero JS by default.
import { defineConfig } from 'astro/config';
import sitemap from '@astrojs/sitemap';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  site: 'https://getinkwell.vercel.app',
  output: 'static',
  // Astro 7 defaults to JSX whitespace rules, which silently delete a line break between text and an
  // inline element ("you can" + <a> became "you canbuy me a coffee"). `true` compresses losslessly, so
  // prose can wrap across source lines like ordinary HTML.
  compressHTML: true,
  // One small stylesheet: inline it, so the first paint doesn't wait on a second request.
  build: { inlineStylesheets: 'always' },
  integrations: [
    // Only real, indexable pages: the 404 is served with a 404 status and stays out of the sitemap.
    sitemap({ filter: (page) => !/\/404\/?$/.test(page), lastmod: new Date() }),
  ],
  vite: { plugins: [tailwindcss()] },
});
