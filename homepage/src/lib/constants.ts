/**
 * Every outbound link and product fact the page states lives here once.
 * Facts come from the Inkwell 1.0 README and docs/MODEL-WEIGHTS.md. Where an older version of the
 * site said something the README contradicts, the README won.
 */

/**
 * The downloadable release. Advertise only a version that already exists on the releases page:
 * bump it after a release is published, never before, then run `node scripts/snapshot-release.mjs`
 * so the download links follow (see ./release.ts).
 */
export const APP_VERSION = "1.0.1";

/** Canonical origin: the Vercel project alias until an owned domain exists. */
export const SITE_URL = "https://getinkwell.vercel.app";

export const GITHUB_URL = "https://github.com/SirSicard/inkwell";
export const RELEASES_URL = `${GITHUB_URL}/releases`;
export const RELEASES_LATEST_URL = `${GITHUB_URL}/releases/latest`;
export const ISSUES_URL = `${GITHUB_URL}/issues`;
export const LICENSE_URL = `${GITHUB_URL}/blob/main/LICENSE`;
export const BUILD_FROM_SOURCE_URL = `${GITHUB_URL}#build-from-source`;
export const CONTRIBUTING_URL = `${GITHUB_URL}/blob/main/CONTRIBUTING.md`;
export const SECURITY_URL = `${GITHUB_URL}/blob/main/SECURITY.md`;

/** The README's "Support the project": the tip link must stay on the page. */
export const DONATION_URL = "https://buymeacoffee.com/mattiasherzig";
export const DONATION_SUGGESTED = "€10";

export const AUTHOR_NAME = "Mattias Hjemgaard";
export const AUTHOR_URL = "https://github.com/SirSicard"; // the maintainer's GitHub profile
export const MAINTAINER_HANDLE = "SirSicard";
export const MAINTAINER_GITHUB_URL = "https://github.com/SirSicard";

export const HANDY_URL = "https://github.com/cjpais/Handy";
export const HUGGING_FACE_URL = "https://huggingface.co";
export const GROQ_KEYS_URL = "https://console.groq.com/keys";
export const GROQ_RATE_LIMITS_URL = "https://console.groq.com/docs/rate-limits";
export const APP_SDK_NOTE_URL = `${GITHUB_URL}/blob/main/windows/WINDOWS-APP-SDK-TELEMETRY.md`;
export const MODEL_WEIGHTS_URL = `${GITHUB_URL}/blob/main/docs/MODEL-WEIGHTS.md`;
export const THIRD_PARTY_URL = `${GITHUB_URL}/blob/main/THIRD_PARTY.md`;

/**
 * Inkwell 0.2, the last version for Intel Macs and Linux (README: "Intel Macs and Linux get no 1.0").
 * The branch it lives on, and its last release: bump the tag if a 0.2.10 is published.
 */
export const LEGACY_BRANCH_URL = `${GITHUB_URL}/tree/legacy/0.2`;
export const LEGACY_RELEASE_URL = `${RELEASES_URL}/tag/v0.2.9`;

/** Windows code signing: the SignPath Foundation programme requires its policy on the homepage. */
export const SIGNPATH_URL = "https://signpath.io";
export const SIGNPATH_FOUNDATION_URL = "https://signpath.org";

/** What Inkwell 1.0 runs on (README "Credits"). */
export const CREDITS = [
  { name: "llama.cpp", url: "https://github.com/ggml-org/llama.cpp" },
  { name: "FluidAudio", url: "https://github.com/FluidInference/FluidAudio" },
  { name: "sherpa-onnx", url: "https://github.com/k2-fsa/sherpa-onnx" },
  { name: "NeMo-Speech.cpp", url: "https://github.com/NVIDIA/NeMo-Speech.cpp" },
  { name: "Silero VAD", url: "https://github.com/snakers4/silero-vad" },
  { name: "tract", url: "https://github.com/sonos/tract" },
  { name: "Sparkle", url: "https://github.com/sparkle-project/Sparkle" },
  { name: "Velopack", url: "https://github.com/velopack/velopack" },
] as const;

/**
 * The models Inkwell 1.0 downloads, from docs/MODEL-WEIGHTS.md ("Models in 1.0") and the README's
 * Models table: name, job, size and licence. No accuracy figures: 1.0 publishes none.
 */
export const MODELS = [
  { name: "Qwen3-ASR 1.7B", job: "Dictation, and the meeting’s final pass", size: "2.5 GB", licence: "Apache-2.0" },
  { name: "Parakeet TDT 0.6B v3", job: "Live words; dictation on a PC without a GPU. 0.48 GB on the Mac, 0.67 GB on Windows", size: "0.48–0.67 GB", licence: "CC-BY-4.0" },
  { name: "Nemotron-3-Diarization", job: "Who said what on the far end", size: "0.11 GB", licence: "OpenMDW-1.1" },
  { name: "Silero VAD v6.2.3", job: "Voice activity", size: "1.3 MB", licence: "MIT" },
  { name: "Qwen3-4B-Instruct-2507", job: "Polish, voice edit, summaries and Ask on a PC (optional)", size: "2.5 GB", licence: "Apache-2.0" },
] as const;
