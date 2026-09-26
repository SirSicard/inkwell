import Reveal from "./Reveal";
import SectionHeading from "./SectionHeading";

/*
  Sources: src-tauri/src/pipeline.rs (capture → resample → local transcribe →
  paste), src-tauri/src/settings.rs (debug_save_audio opt-in, plaintext secrets
  stripped from settings.json), src-tauri/src/history.rs + setup.rs (SQLite in
  the app data dir), src-tauri/src/llm.rs (BYOK providers, keyring lookup),
  src-tauri/tauri.conf.json (updater endpoint), src-tauri/src/commands.rs
  (`hf_base`; model files come from Hugging Face), src-tauri/src/setup.rs
  (SILERO_VAD_URL; the VAD model comes from sherpa-onnx's GitHub releases),
  src-tauri/src/voiceedit.rs (voice editing uses the same BYOK provider),
  inkwell-updater/src/index.ts (reads KV, logs nothing), src/App.tsx (the update
  check is a 5000 ms setTimeout on mount; no setting disables it).

  "Only three network calls" is meant literally: grepping every http(s) URL in
  src-tauri/src yields exactly these destinations: Hugging Face model repos and
  one GitHub release asset (downloads), the updater worker, and the BYOK LLM
  endpoints. Fonts are local
  (public/fonts) and the Tauri CSP has no remote connect-src.
*/

const local = [
  "Your voice. Audio is captured, resampled to 16 kHz and transcribed in memory by a model on your own CPU. It is never uploaded.",
  "Your transcripts. History lives in a SQLite file in Inkwell's app data directory. Nothing syncs anywhere.",
  "Your settings, snippets, dictionary and voice commands. Plain files on your disk.",
  "Your API key, if you add one. Stored in the OS keyring, never in a config file, and any key an older build wrote in plain text is stripped the next time settings load.",
];

const leaves = [
  {
    title: "Model downloads",
    body: "When you choose a model, Inkwell pulls the files from Hugging Face, and on first run it fetches the small voice-detection model from the sherpa-onnx releases on GitHub. Plain file downloads; nothing about you goes with them.",
  },
  {
    title: "Update checks",
    body: "Five seconds after launch the app asks the release endpoint whether a newer version exists, and shows a dismissible toast if so. The request carries the app version, OS and CPU architecture, and the endpoint only reads the latest release record. It runs automatically; there is no switch for it yet.",
  },
  {
    title: "AI polish and voice editing, only if you use them",
    body: "Off by default. When you use them, the text (never the audio) goes straight from your machine to the provider whose key you supplied. There is no Inkwell server in the middle: earlier builds had a free proxy tier and it has been removed. Bring your own key or the feature does nothing.",
  },
];

export default function PrivacySection() {
  return (
    <section
      id="privacy"
      aria-labelledby="privacy-title"
      className="mx-auto max-w-5xl scroll-mt-16 px-6 py-24 sm:py-28"
    >
      <SectionHeading
        eyebrow="Privacy"
        id="privacy-title"
        title="What stays, and what can leave."
        intro="No account. No telemetry. No analytics. No crash reporting. Here is the whole picture."
      />

      <div className="grid gap-4 lg:grid-cols-2">
        <Reveal className="h-full">
          <div className="surface h-full p-6 sm:p-7">
            <h3 className="text-base font-medium">Never leaves your machine</h3>
            <ul className="mt-4 space-y-3">
              {local.map((item) => (
                <li
                  key={item}
                  className="flex gap-3 text-sm leading-relaxed"
                  style={{ color: "var(--text-secondary)" }}
                >
                  <span
                    aria-hidden="true"
                    className="mt-2 h-1.5 w-1.5 shrink-0 rounded-full"
                    style={{ background: "var(--text-tertiary)" }}
                  />
                  <span>{item}</span>
                </li>
              ))}
            </ul>
            <p
              className="mt-5 border-t pt-4 text-sm leading-relaxed"
              style={{
                borderColor: "var(--border)",
                color: "var(--text-tertiary)",
              }}
            >
              One exception, and it is yours to make: a debug setting can write
              each dictation&rsquo;s audio to a temp folder. It is off unless you
              switch it on.
            </p>
          </div>
        </Reveal>

        <Reveal delay={80} className="h-full">
          <div className="surface h-full p-6 sm:p-7">
            <h3 className="text-base font-medium">
              The only three network calls
            </h3>
            <ol className="mt-4 space-y-4">
              {leaves.map((item, i) => (
                <li key={item.title} className="flex gap-3">
                  <span
                    aria-hidden="true"
                    className="mt-0.5 font-mono text-xs"
                    style={{ color: "var(--accent)" }}
                  >
                    {String(i + 1).padStart(2, "0")}
                  </span>
                  <span>
                    <span className="block text-sm font-medium">
                      {item.title}
                    </span>
                    <span
                      className="mt-1 block text-sm leading-relaxed"
                      style={{ color: "var(--text-secondary)" }}
                    >
                      {item.body}
                    </span>
                  </span>
                </li>
              ))}
            </ol>
            <p
              className="mt-5 border-t pt-4 text-sm leading-relaxed"
              style={{
                borderColor: "var(--border)",
                color: "var(--text-tertiary)",
              }}
            >
              With polish and voice editing unused and your models already on
              disk, the update check is the only thing Inkwell sends, and
              dictation itself keeps working with the network unplugged.
            </p>
          </div>
        </Reveal>
      </div>
    </section>
  );
}
