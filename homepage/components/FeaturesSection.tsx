import type { ReactNode } from "react";
import Reveal from "./Reveal";
import SectionHeading from "./SectionHeading";
import {
  FileIcon,
  KeyboardIcon,
  PenIcon,
  TrayIcon,
  WaveIcon,
  LockIcon,
} from "./icons";

/*
  Every claim below is checked against the app source:
  - model catalogue: src/tabs/ModelsTab.tsx (MODEL_CATALOG, 13 entries) cross-checked
    against the `hf_base` match in src-tauri/src/commands.rs; only 12 have a download
    URL. moonshine-tiny falls through to `Err("No download URL")`, so it is never
    offered here as something a user can actually get.
  - CPU-only inference: src-tauri/src/engine.rs (PROVIDER is the constant "cpu")
  - styles: src-tauri/src/style.rs (Formal / Casual / Relaxed)
  - dictionary: src-tauri/src/dictionary.rs (case-insensitive, word boundary)
  - snippets + variables: src-tauri/src/snippets.rs ({date} {time} {clipboard})
  - voice commands: src-tauri/src/voicecommand.rs (wake prefix, default off)
  - file formats: src-tauri/src/filetranscribe.rs (SUPPORTED_EXTENSIONS)
  - export formats: src-tauri/src/commands.rs (txt | srt | json | csv)
  - tray + overlay: src-tauri/src/tray.rs, src-tauri/src/overlay.rs
  - clipboard restore: src-tauri/src/paste.rs (text only; arboard cannot
    round-trip images or custom flavours, so those stay replaced)
  - polish providers: src-tauri/src/llm.rs (BYOK only, key in OS keyring)
  - Advanced Mode gate: src/types.ts (basicTabs is Dashboard/General/About only)
    and src-tauri/src/settings.rs (advanced_mode defaults to false)
  - per-app styles: src-tauri/src/appdetect.rs (get_foreground_app_id is
    implemented for Windows and macOS; the Linux cfg branch returns None)
*/
const features: { icon: ReactNode; title: string; body: string }[] = [
  {
    icon: <WaveIcon />,
    title: "Twelve local models",
    body: "Parakeet V3 (670 MB, 25 European languages) is the default. Whisper spans 99 languages from a 98 MB Tiny to a 1.5 GB Large V3, and SenseVoice covers Chinese, English, Japanese, Korean and Cantonese in 160 MB. Download, switch and delete them inside the app. Inference is CPU only.",
  },
  {
    icon: <PenIcon />,
    title: "Formatting without a model",
    body: "Three styles: Formal capitalises and punctuates, Casual keeps it light, Relaxed strips it back to lowercase. Per-app rules can swap style by whatever app is in front (formal in Outlook, relaxed in a terminal) on macOS and Windows, off until you enable it. A custom dictionary fixes the names and jargon your model keeps mangling: case-insensitive, matched on word boundaries.",
  },
  {
    icon: <KeyboardIcon />,
    title: "Snippets and voice commands",
    body: "Trigger phrases expand into full blocks of text, with {date}, {time} and {clipboard} filled in as you dictate. Voice commands listen for a wake prefix (“inkwell, scratch that”, “inkwell, formal mode”) and are off until you switch them on.",
  },
  {
    icon: <FileIcon />,
    title: "Files, history and export",
    body: "Drop in audio or video (MP3, WAV, FLAC, OGG, M4A, AAC, MP4, MOV, MKV, WebM, AVI, WMA) and transcribe it with the same local model. Transcripts land in a searchable SQLite history you can export as TXT, SRT, JSON or CSV.",
  },
  {
    icon: <TrayIcon />,
    title: "Out of the way",
    body: "Inkwell sits in the tray; the hotkey works with the window hidden. A small always-on-top ink blob shows that it is listening, and whatever text you had on the clipboard is put back after the paste. An image on the clipboard is not restored; it stays replaced rather than wiped.",
  },
  {
    icon: <LockIcon />,
    title: "Optional AI polish, your key",
    body: "Turn it on and the transcribed text, never the audio, is sent from your machine to a provider you choose: OpenAI, Groq, Anthropic, OpenRouter, or any OpenAI-compatible endpoint including a local one. Your key lives in the OS keyring. Off by default.",
  },
];

/* Stated plainly so nobody has to install the app to find out. */
const notBuilt = [
  "Live text while you speak. Transcription starts when you release the key.",
  "Per-app style overrides on Linux. Foreground-app detection works on macOS and Windows; X11 and Wayland do not hand it over.",
  "GPU acceleration. Everything runs on the CPU.",
  "Speaker labels, meeting mode, calendar integration. Not planned.",
];

export default function FeaturesSection() {
  return (
    <section
      id="features"
      aria-labelledby="features-title"
      className="mx-auto max-w-6xl scroll-mt-16 px-6 py-24 sm:py-28"
    >
      <SectionHeading
        eyebrow="What it does"
        id="features-title"
        title="Everything on this page runs on your hardware."
        intro="A fresh install shows you three tabs. Models, dictionary, snippets, files, per-app styles, voice commands and AI polish appear once you turn on Advanced Mode in General settings: one switch, off by default, so the first run stays a hotkey and nothing else."
      />

      <ul className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {features.map((f, i) => (
          <li key={f.title}>
            <Reveal delay={(i % 3) * 70} className="h-full">
              <div className="surface h-full p-6">
                {/* Icons stay neutral on purpose. The copper accent is reserved
                    for actions, numerals and eyebrows; six tinted glyphs in one
                    grid would make the accent decorative and stop it meaning
                    anything on the download button. */}
                <span style={{ color: "var(--text-tertiary)" }}>{f.icon}</span>
                <h3 className="mt-4 text-base font-medium">{f.title}</h3>
                <p
                  className="mt-2 text-pretty text-sm leading-relaxed"
                  style={{ color: "var(--text-secondary)" }}
                >
                  {f.body}
                </p>
              </div>
            </Reveal>
          </li>
        ))}
      </ul>

      <Reveal className="mt-10">
        <div className="surface-quiet p-6 sm:p-7">
          <h3 className="text-sm font-medium">What it does not do</h3>
          <ul className="mt-4 grid gap-2.5 sm:grid-cols-2">
            {notBuilt.map((item) => (
              <li
                key={item}
                className="flex gap-2.5 text-sm leading-relaxed"
                style={{ color: "var(--text-tertiary)" }}
              >
                <span aria-hidden="true" style={{ color: "var(--border-strong)" }}>
                  ·
                </span>
                <span>{item}</span>
              </li>
            ))}
          </ul>
        </div>
      </Reveal>
    </section>
  );
}
