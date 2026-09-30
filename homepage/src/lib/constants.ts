/**
 * Every outbound link and product fact the page states lives here once.
 * Facts come from the Inkwell README (and the model chart it embeds). Where an older version of the
 * site said something the README contradicts, the README won.
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

export const HANDY_URL = "https://github.com/cjpais/Handy";
export const SHERPA_ONNX_URL = "https://github.com/k2-fsa/sherpa-onnx";
export const SILERO_VAD_URL = "https://github.com/snakers4/silero-vad";
export const TAURI_URL = "https://tauri.app";
export const HUGGING_FACE_URL = "https://huggingface.co";
export const GROQ_CONSOLE_URL = "https://console.groq.com";

/** Windows code signing: the SignPath Foundation programme requires its policy on the homepage. */
export const SIGNPATH_URL = "https://signpath.io";
export const SIGNPATH_FOUNDATION_URL = "https://signpath.org";

/** The update check's host, named in the README's privacy section. */
export const UPDATER_HOST = "inkwell-updater.mattias-e67.workers.dev";

/**
 * The five models. Word error rate, time and size come from the README's model chart
 * (docs/media/models-light.svg: eight recordings of one voice; seconds to transcribe 57 s of audio).
 * Languages and "pick it when" come from the README's models table, lightly shortened; the maker
 * is the one the previous site named. Sorted by word error rate, as the README's chart is.
 */
export const MODELS = [
  { name: "Qwen3 ASR", maker: "Alibaba", wer: 5.6, seconds: 9.2, mb: 940, languages: "30, including Nordic", when: "You want the best accuracy, or you move between English and a Nordic language. The only one here that doesn’t make you choose." },
  { name: "Parakeet V2", maker: "NVIDIA", wer: 8.0, seconds: 3.6, mb: 670, languages: "English", when: "You only dictate in English. Same download as V3, and measurably more accurate." },
  { name: "SenseVoice", maker: "Alibaba", wer: 9.3, seconds: 1.7, mb: 240, languages: "en, zh, ja, ko, yue", when: "Small disk, slow connection or an older machine. A quarter of the size, the fastest here, and as accurate as Whisper." },
  { name: "Whisper Turbo", maker: "OpenAI", wer: 9.3, seconds: 19.8, mb: 800, languages: "99", when: "You need a language the others don’t reach. Nothing else recommends it." },
  { name: "Parakeet V3", maker: "NVIDIA", wer: 10.5, seconds: 3.7, mb: 670, languages: "25 European", when: "The default. You switch between European languages, or want the language detected for you." },
] as const;

/** Chart scales: a shared zero baseline and a round maximum just above the largest figure. */
export const WER_SCALE_MAX = 12;
export const SECONDS_SCALE_MAX = 20;

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
