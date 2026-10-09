// The download links, against a 1.0.0 release as the release workflows publish it and against the
// 0.2.9 snapshot the page builds from today. Run with `npm test` (Node 23.6+ strips the types).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolveDownloads } from '../src/lib/release-assets.ts';

const base = 'https://github.com/SirSicard/inkwell/releases/download/v1.0.0/';
const hex = 'ab'.repeat(32);
// Every file a 1.0.0 release carries: mac-release.yml (dmg, appcast, build manifest) and
// win-release.yml (setup, Velopack package, its feed, the sums), named by the release-version.sh
// scripts. Sizes are invented.
const v100 = {
  tag: 'v1.0.0',
  publishedAt: '2026-10-20T10:00:00Z',
  assets: [
    ['Inkwell_1.0.0_aarch64.dmg', 61_234_567],
    ['Inkwell_1.0.0_build-manifest.txt', 4_321],
    ['appcast.xml', 2_345],
    ['Inkwell_1.0.0_x64-setup.exe', 98_765_432],
    ['Inkwell_1.0.0_windows-sha256.txt', 321],
    ['InkwellApp-1.0.0-full.nupkg', 97_000_000],
    ['releases.win.json', 456],
  ].map(([name, size]) => ({ name, size, url: base + name, digest: `sha256:${hex}` })),
};

test('a 1.0.0 release resolves the dmg, the installer and the sums file', () => {
  const d = resolveDownloads('1.0.0', v100);
  assert.equal(d.mac.url, base + 'Inkwell_1.0.0_aarch64.dmg');
  assert.equal(d.mac.size, '61.2 MB');
  assert.equal(d.windows.url, base + 'Inkwell_1.0.0_x64-setup.exe');
  assert.equal(d.windows.sha256, hex);
  assert.equal(d.windowsSums?.url, base + 'Inkwell_1.0.0_windows-sha256.txt');
});

test('the page links to no updater file, manifest or legacy build', () => {
  const urls = Object.values(resolveDownloads('1.0.0', v100)).map((a) => a.url);
  for (const u of urls) assert.doesNotMatch(u, /nupkg|releases\.win\.json|appcast|build-manifest|x64\.dmg|\.msi|AppImage|\.deb/);
});

test('a 1.0.0 release without the sums file fails the build', () => {
  const assets = v100.assets.filter((a) => !a.name.endsWith('windows-sha256.txt'));
  assert.throws(() => resolveDownloads('1.0.0', { ...v100, assets }), /windows-sha256\.txt is not an asset/);
});

test('a missing dmg or installer fails the build', () => {
  for (const gone of ['Inkwell_1.0.0_aarch64.dmg', 'Inkwell_1.0.0_x64-setup.exe']) {
    const assets = v100.assets.filter((a) => a.name !== gone);
    assert.throws(() => resolveDownloads('1.0.0', { ...v100, assets }), new RegExp(gone.replace(/\./g, '\\.')));
  }
});

test('a snapshot of another release fails the build', () => {
  assert.throws(() => resolveDownloads('1.0.1', v100), /release\.json is v1\.0\.0, APP_VERSION is 1\.0\.1/);
});

test('an asset whose URL points elsewhere is refused', () => {
  const assets = v100.assets.map((a) => (a.name.endsWith('.dmg') ? { ...a, url: 'https://example.com/x.dmg' } : a));
  assert.throws(() => resolveDownloads('1.0.0', { ...v100, assets }), /aarch64\.dmg is not an asset/);
});

test('a digest that is not SHA-256 hex is dropped, not shown', () => {
  const assets = v100.assets.map((a) => ({ ...a, digest: 'sha512:zz' }));
  assert.equal(resolveDownloads('1.0.0', { ...v100, assets }).windows.sha256, undefined);
});

test('the 0.2.9 snapshot in the repo still builds, without a sums file', () => {
  const snap = JSON.parse(readFileSync(new URL('../src/data/release.json', import.meta.url), 'utf8'));
  const constants = readFileSync(new URL('../src/lib/constants.ts', import.meta.url), 'utf8');
  const version = /APP_VERSION\s*=\s*"([^"]+)"/.exec(constants)[1];
  const d = resolveDownloads(version, snap);
  assert.match(d.mac.name, /_aarch64\.dmg$/);
  assert.match(d.windows.name, /_x64-setup\.exe$/);
  if (version.startsWith('0.')) assert.equal(d.windowsSums, undefined);
});
