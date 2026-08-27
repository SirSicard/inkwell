/**
 * Single source of truth for every outbound link and support detail on the site.
 * Nothing here may be hardcoded a second time in a component.
 */

/** Public repository. */
export const GITHUB_URL = "https://github.com/SirSicard/inkwell";
export const GITHUB_RELEASES_URL = `${GITHUB_URL}/releases`;
export const GITHUB_RELEASES_LATEST_URL = `${GITHUB_URL}/releases/latest`;
export const GITHUB_ISSUES_URL = `${GITHUB_URL}/issues`;
export const GITHUB_LICENSE_URL = `${GITHUB_URL}/blob/main/LICENSE`;
export const GITHUB_README_URL = `${GITHUB_URL}/blob/main/README.md`;

/**
 * The owner's verified Buy Me a Coffee page. Must match DONATION_URL in the
 * app's src/constants.ts; there is no build-time check tying them together.
 */
export const DONATION_URL = "https://buymeacoffee.com/mattiasherzig";

/** Suggested (entirely voluntary) tip. Nothing in the app is gated behind it. */
export const DONATION_SUGGESTED = "€10";

/**
 * Canonical site origin. The Vercel project alias until an owned domain
 * exists; it drives the canonical URL and the absolute OG image URL in
 * app/layout.tsx. Update when the real domain lands.
 */
export const SITE_URL = "https://getinkwell.vercel.app";

/**
 * Matches `version` in src-tauri/tauri.conf.json. Bump both together.
 *
 * The site may only be deployed while a release with this exact version exists,
 * because every "Download" link points at /releases/latest and the page
 * describes the software it hands out. That held this constant at 0.1.1 (a
 * pre-rehaul build with agent mode and the free proxy tier) until v0.2.0 was
 * cut on 2026-07-30.
 */
export const APP_VERSION = "0.2.9";

/** Author. */
export const AUTHOR_NAME = "Mattias Herzig";
export const AUTHOR_URL = "https://mattiasherzig.com";

/** Upstream projects we lean on, credited in the footer. */
export const SHERPA_ONNX_URL = "https://github.com/k2-fsa/sherpa-onnx";
export const TAURI_URL = "https://tauri.app";
export const HANDY_URL = "https://github.com/cjpais/Handy";
/** Silero VAD supplies silero_vad.onnx, used in src-tauri/src/vad.rs. */
export const SILERO_VAD_URL = "https://github.com/snakers4/silero-vad";
