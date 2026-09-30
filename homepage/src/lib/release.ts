/**
 * Direct download links, one per platform, each built from that platform's version and checked
 * against its release's real asset list (src/data/release.json, copied from GitHub by
 * scripts/snapshot-release.mjs). The build fails rather than ship a link to a file that doesn't exist.
 *
 * Server-side only: the page writes these into the HTML. The client script never imports this file,
 * so the snapshot is not shipped as JavaScript.
 */
import snapshot from '../data/release.json';
import { MAC_VERSION, RELEASES_URL, WINDOWS_VERSION } from './constants';

export type Asset = { name: string; url: string; size: string };

function asset(version: string, suffix: string): Asset {
  const tag = `v${version}`;
  const release = snapshot.releases.find((r) => r.tag === tag);
  if (!release) throw new Error(`src/data/release.json has no ${tag}: run scripts/snapshot-release.mjs`);
  const name = `Inkwell_${version}_${suffix}`;
  const url = `${RELEASES_URL}/download/${tag}/${name}`;
  const found = release.assets.find((a) => a.name === name);
  if (!found || found.url !== url) throw new Error(`${name} is not an asset of ${tag}`);
  return { name, url, size: `${(found.size / 1e6).toFixed(1)}\u00a0MB` }; // no-break space: the unit never wraps away from its number
}

/**
 * The Mac's disk image is named by mac/scripts/release-version.sh. The Windows installer's name must
 * match what the Windows release workflow uploads (0.2's NSIS installer used this one).
 */
export const DOWNLOADS = {
  macOS: asset(MAC_VERSION, 'aarch64.dmg'),
  windows: asset(WINDOWS_VERSION, 'x64-setup.exe'),
} as const;

/**
 * What the primary button offers per detected OS: the file, and the sentence that replaces the version
 * note under it. Undetected and phone visitors, and Linux, keep the releases/latest page.
 */
export const PRIMARY = {
  macOS: { href: DOWNLOADS.macOS.url, note: `Version ${MAC_VERSION} for Macs with Apple silicon on macOS 26 or later: the disk image, ${DOWNLOADS.macOS.size}.` },
  Windows: { href: DOWNLOADS.windows.url, note: `Version ${WINDOWS_VERSION} for Windows 11 24H2 or later on x64: the installer, ${DOWNLOADS.windows.size}. It is not code signed yet; the install steps are below.` },
} as const;
