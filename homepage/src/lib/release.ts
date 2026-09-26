/**
 * Direct download links, built from APP_VERSION and checked against the release's real asset list
 * (src/data/release.json, copied from GitHub by scripts/snapshot-release.mjs). The build fails
 * rather than ship a link to a file that doesn't exist.
 *
 * Server-side only: the page writes these into the HTML. The client script never imports this file,
 * so the snapshot is not shipped as JavaScript.
 */
import release from '../data/release.json';
import { APP_VERSION, RELEASES_URL } from './constants';

export type Asset = { name: string; url: string; size: string };

function asset(suffix: string): Asset {
  const name = `Inkwell_${APP_VERSION}_${suffix}`;
  const url = `${RELEASES_URL}/download/v${APP_VERSION}/${name}`;
  const found = release.assets.find((a) => a.name === name);
  if (release.tag !== `v${APP_VERSION}`) {
    throw new Error(`src/data/release.json is ${release.tag}, APP_VERSION is ${APP_VERSION}: run scripts/snapshot-release.mjs`);
  }
  if (!found || found.url !== url) throw new Error(`${name} is not an asset of ${release.tag}`);
  return { name, url, size: `${(found.size / 1e6).toFixed(1)}\u00a0MB` }; // no-break space: the unit never wraps away from its number
}

export const DOWNLOADS = {
  macOS: asset('aarch64.dmg'),
  macOSIntel: asset('x64.dmg'),
  windows: asset('x64-setup.exe'),
  windowsMsi: asset('x64_en-US.msi'),
  appImage: asset('amd64.AppImage'),
  deb: asset('amd64.deb'),
} as const;

/**
 * What the primary button offers per detected OS: the file, and the sentence that replaces the version
 * note under it. Undetected and phone visitors keep the releases/latest page.
 */
export const PRIMARY = {
  macOS: { href: DOWNLOADS.macOS.url, note: `Version ${APP_VERSION} for Macs with Apple Silicon: the disk image, ${DOWNLOADS.macOS.size}. The Intel build is in the list below.` },
  // Offered only when Chromium reports an x86 Mac (platform.ts). Safari and Firefox don't expose the
  // architecture, so they keep the Apple Silicon file, and the Intel build stays one link away.
  macOSIntel: { href: DOWNLOADS.macOSIntel.url, note: `Version ${APP_VERSION} for Macs with Intel: the disk image, ${DOWNLOADS.macOSIntel.size}.` },
  Windows: { href: DOWNLOADS.windows.url, note: `Version ${APP_VERSION} for Windows 10 and 11: the installer, ${DOWNLOADS.windows.size}.` },
  Linux: { href: DOWNLOADS.appImage.url, note: `Version ${APP_VERSION} for Linux: the AppImage, ${DOWNLOADS.appImage.size}. The .deb is in the download list below.` },
} as const;
