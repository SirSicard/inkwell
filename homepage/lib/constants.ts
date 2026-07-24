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
 * DONATION LINK — PLACEHOLDER.
 *
 * The owner MUST replace `REPLACE_ME` with the real Buy Me a Coffee handle
 * before this site goes live. Until then this URL resolves to a 404 page.
 * Do not guess a handle: an invented-but-plausible donation URL could send
 * money to a stranger.
 */
export const DONATION_URL = "https://buymeacoffee.com/REPLACE_ME";

/** Suggested (entirely voluntary) tip. Nothing in the app is gated behind it. */
export const DONATION_SUGGESTED = "€10";

/**
 * Canonical site origin — PLACEHOLDER.
 * Replace with the real domain once the site is deployed; it drives the
 * canonical URL and the absolute OG image URL in app/layout.tsx.
 */
export const SITE_URL = "https://inkwell.example";

/**
 * Matches `version` in src-tauri/tauri.conf.json. Bump both together.
 *
 * ⚠ SHIP BLOCKER — do not deploy this site until a release exists that matches
 * this copy. The rehaul (agent mode removed, free polish proxy removed, BYOK
 * only) is still under `## [Unreleased]` in CHANGELOG.md. The newest *published*
 * release is v0.1.1 from 2026-04-01, which predates all of it: that build still
 * contains Voice Agent mode and the free proxy tier this page says do not exist.
 * Every "Download" link points at /releases/latest, so shipping now would hand
 * visitors the exact software the page disclaims. Cut a release (0.2.0), bump
 * tauri.conf.json and this constant, then deploy.
 */
export const APP_VERSION = "0.1.1";

/** Author. */
export const AUTHOR_NAME = "Mattias Herzig";
export const AUTHOR_URL = "https://mattiasherzig.com";

/** Upstream projects we lean on, credited in the footer. */
export const SHERPA_ONNX_URL = "https://github.com/k2-fsa/sherpa-onnx";
export const TAURI_URL = "https://tauri.app";
export const HANDY_URL = "https://github.com/cjpais/Handy";
/** Silero VAD supplies silero_vad.onnx, used in src-tauri/src/vad.rs. */
export const SILERO_VAD_URL = "https://github.com/snakers4/silero-vad";
