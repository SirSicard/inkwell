/**
 * The release's files the page links to, resolved against a snapshot of the release's real asset
 * list (src/data/release.json, from scripts/snapshot-release.mjs). Pure and import-free, so the
 * tests (tests/release.test.mjs) run it under plain Node.
 *
 * Inkwell 1.x ships one Mac file and one Windows file, plus files the page must never offer:
 *   Inkwell_X.Y.Z_aarch64.dmg            Mac, Apple silicon (mac/scripts/release-version.sh)
 *   Inkwell_X.Y.Z_build-manifest.txt     the Mac build's record (not linked)
 *   appcast.xml                          Sparkle's feed (not linked)
 *   Inkwell_X.Y.Z_x64-setup.exe          Windows 11, x64 (windows/scripts/release-version.sh)
 *   Inkwell_X.Y.Z_windows-sha256.txt     the Windows files' SHA-256 sums
 *   InkwellApp-X.Y.Z-full.nupkg          Velopack's package (not linked)
 *   releases.win.json                    Velopack's feed (not linked)
 * There is no Intel dmg, MSI, AppImage or .deb in 1.x: Intel Macs and Linux stay on 0.2.
 */

export type SnapshotAsset = { name: string; size: number; url: string; digest?: string };
export type Snapshot = { tag: string; publishedAt?: string; assets: SnapshotAsset[] };

export type Asset = {
  name: string;
  url: string;
  /** "123.4 MB", with a no-break space so the unit never wraps away from its number. */
  size: string;
  /** Lowercase hex SHA-256 from GitHub's record of the upload, when the snapshot has it. */
  sha256?: string;
};

export type Downloads = {
  mac: Asset;
  windows: Asset;
  /** The Windows sums file: required from 1.0, absent from 0.2 releases. */
  windowsSums?: Asset;
};

const RELEASES = 'https://github.com/SirSicard/inkwell/releases';

function major(version: string): number {
  const m = /^(\d+)\.(\d+)\.(\d+)$/.exec(version);
  if (!m) throw new Error(`APP_VERSION "${version}" is not X.Y.Z`);
  return Number(m[1]);
}

/** The page's download links for `version`, or an error naming what the release lacks. */
export function resolveDownloads(version: string, snapshot: Snapshot): Downloads {
  const is1x = major(version) >= 1;
  if (snapshot.tag !== `v${version}`) {
    throw new Error(`src/data/release.json is ${snapshot.tag}, APP_VERSION is ${version}: run scripts/snapshot-release.mjs`);
  }
  const find = (name: string, required: boolean): Asset | undefined => {
    const url = `${RELEASES}/download/v${version}/${name}`;
    const found = snapshot.assets.find((a) => a.name === name);
    if (!found || found.url !== url) {
      if (required) throw new Error(`${name} is not an asset of ${snapshot.tag}`);
      return undefined;
    }
    const sha = found.digest?.startsWith('sha256:') ? found.digest.slice(7).toLowerCase() : undefined;
    return {
      name,
      url,
      size: `${(found.size / 1e6).toFixed(1)} MB`,
      ...(sha && /^[0-9a-f]{64}$/.test(sha) ? { sha256: sha } : {}),
    };
  };
  const mac = find(`Inkwell_${version}_aarch64.dmg`, true)!;
  const windows = find(`Inkwell_${version}_x64-setup.exe`, true)!;
  const windowsSums = find(`Inkwell_${version}_windows-sha256.txt`, is1x);
  return windowsSums ? { mac, windows, windowsSums } : { mac, windows };
}
