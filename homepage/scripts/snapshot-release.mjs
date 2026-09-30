// Copies each advertised release's real asset names and sizes into src/data/release.json, so the
// download links are built from what GitHub actually published, never from typed-out file names.
// Run after bumping MAC_VERSION or WINDOWS_VERSION (and only once that release exists):
//   node scripts/snapshot-release.mjs
// Needs the GitHub CLI (`gh`), read-only. One entry per distinct version.
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';

const constants = readFileSync(new URL('../src/lib/constants.ts', import.meta.url), 'utf8');
const versions = ['MAC_VERSION', 'WINDOWS_VERSION'].map((key) => {
  const version = new RegExp(`${key}\\s*=\\s*"([^"]+)"`).exec(constants)?.[1];
  if (!version) throw new Error(`${key} not found in src/lib/constants.ts`);
  return version;
});

const releases = [...new Set(versions)].map((version) => {
  const raw = execFileSync(
    'gh',
    ['release', 'view', `v${version}`, '-R', 'SirSicard/inkwell', '--json', 'tagName,publishedAt,assets'],
    { encoding: 'utf8' },
  );
  const r = JSON.parse(raw);
  return {
    tag: r.tagName,
    publishedAt: r.publishedAt,
    assets: r.assets.map((a) => ({ name: a.name, size: a.size, url: a.url })),
  };
});
writeFileSync(new URL('../src/data/release.json', import.meta.url), JSON.stringify({ releases }, null, 2) + '\n');
console.log(`wrote src/data/release.json: ${releases.map((r) => `${r.tag}, ${r.assets.length} assets`).join('; ')}`);
