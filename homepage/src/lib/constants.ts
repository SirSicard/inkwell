/**
 * Every outbound link and product fact the page states lives here once.
 * Facts come from the Inkwell README. Where an older version of the site said something the README
 * contradicts, the README won.
 */

/**
 * The downloadable release, per platform. Advertise only a version that already exists on the
 * releases page: bump it after that release is published, never before, then run
 * `node scripts/snapshot-release.mjs` so the download links follow (see ./release.ts). Each platform
 * names its own version, so one can move to a new release without the other's file having to be in it.
 */
export const MAC_VERSION = "0.2.9";
export const WINDOWS_VERSION = "0.2.9";
/** The one version both platforms offer, or null while they differ. */
export const SHARED_VERSION: string | null = MAC_VERSION === WINDOWS_VERSION ? MAC_VERSION : null;

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

/** Must match the README's "Support the project" link. */
export const DONATION_URL = "https://buymeacoffee.com/mattiasherzig";
export const DONATION_SUGGESTED = "€10";

export const AUTHOR_NAME = "Mattias Hjemgaard";
export const AUTHOR_URL = "https://github.com/SirSicard"; // the maintainer's GitHub profile
export const MAINTAINER_HANDLE = "SirSicard";
export const MAINTAINER_GITHUB_URL = "https://github.com/SirSicard";

export const MODEL_WEIGHTS_URL = `${GITHUB_URL}/blob/main/docs/MODEL-WEIGHTS.md`;

export const HANDY_URL = "https://github.com/cjpais/Handy";
export const LLAMA_CPP_URL = "https://github.com/ggml-org/llama.cpp";
export const FLUIDAUDIO_URL = "https://github.com/FluidInference/FluidAudio";
export const NEMO_SPEECH_URL = "https://github.com/NVIDIA/NeMo-Speech.cpp";
export const SILERO_VAD_URL = "https://github.com/snakers4/silero-vad";
export const HUGGING_FACE_URL = "https://huggingface.co";

/** Windows code signing: the SignPath Foundation programme requires its policy on the homepage. */
export const SIGNPATH_URL = "https://signpath.io";
export const SIGNPATH_FOUNDATION_URL = "https://signpath.org";

/**
 * The models 1.0 runs, one per job, from docs/MODEL-WEIGHTS.md (with its sizes, where it gives one) and
 * the README's models table. The maker is the model's publisher.
 */
export const MODELS = [
  { name: "Qwen3-ASR 1.7B", maker: "Alibaba", job: "Writes your dictations, and every meeting’s transcript.", runsOn: "llama.cpp: Metal on the Mac, Vulkan or the CPU on Windows", size: "2.5 GB" },
  { name: "Parakeet TDT 0.6B v3", maker: "NVIDIA", job: "Shows your words while you are still speaking. On a Windows PC without a GPU, it writes the dictations too.", runsOn: "FluidAudio on the Mac’s Neural Engine; sherpa-onnx on the CPU on Windows", size: null },
  { name: "Nemotron-3-Diarization", maker: "NVIDIA", job: "Tells the other side’s speakers apart when a meeting ends.", runsOn: "NeMo-Speech.cpp: Metal on the Mac, Vulkan or the CPU on Windows", size: "0.11 GB" },
  { name: "Silero VAD", maker: "Silero", job: "Finds where speech starts and stops.", runsOn: "tract, in Rust", size: "1.3 MB" },
] as const;

/**
 * The hotkey demo's sentences. They are transcripts visible in the app's own dashboard screenshot.
 * The first is server-rendered as already landed.
 * PLACEHOLDER for a real recording and its real transcript.
 */
export const DEMO_SENTENCES = [
  "Can you take a look at the pull request when you get a chance? No rush.",
  "Let's ship the release notes today and pick this up again on Monday.",
  "Reminder to myself: the parser needs a test for the empty input case.",
  "Two things before the demo: check the microphone, and mute notifications.",
];
