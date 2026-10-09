// Copies the release's real asset names and sizes into src/data/release.json, so the download
// links are built from what GitHub actually published, never from typed-out file names.
// Run after bumping APP_VERSION (and only once that release exists):
//   node scripts/snapshot-release.mjs
// Needs the GitHub CLI (`gh`), read-only.
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';

const constants = readFileSync(new URL('../src/lib/constants.ts', import.meta.url), 'utf8');
const version = /APP_VERSION\s*=\s*"([^"]+)"/.exec(constants)?.[1];
if (!version) throw new Error('APP_VERSION not found in src/lib/constants.ts');

const raw = execFileSync(
  'gh',
  ['release', 'view', `v${version}`, '-R', 'SirSicard/inkwell', '--json', 'tagName,publishedAt,assets'],
  { encoding: 'utf8' },
);
const r = JSON.parse(raw);
const out = {
  tag: r.tagName,
  publishedAt: r.publishedAt,
  // digest: GitHub's SHA-256 of the upload; the page shows the Windows installer's beside its check.
  assets: r.assets.map((a) => ({ name: a.name, size: a.size, url: a.url, ...(a.digest ? { digest: a.digest } : {}) })),
};
writeFileSync(new URL('../src/data/release.json', import.meta.url), JSON.stringify(out, null, 2) + '\n');
console.log(`wrote src/data/release.json: ${out.tag}, ${out.assets.length} assets`);
