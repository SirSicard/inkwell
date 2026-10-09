/**
 * Direct download links, built from APP_VERSION and checked against the release's real asset list
 * (src/data/release.json, copied from GitHub by scripts/snapshot-release.mjs). The build fails
 * rather than ship a link to a file that doesn't exist. The rules are in ./release-assets.ts.
 *
 * Server-side only: the page writes these into the HTML. The client script never imports this file,
 * so the snapshot is not shipped as JavaScript.
 */
import release from '../data/release.json';
import { APP_VERSION } from './constants';
import { resolveDownloads, type Snapshot } from './release-assets';

export type { Asset } from './release-assets';

export const DOWNLOADS = resolveDownloads(APP_VERSION, release as Snapshot);
