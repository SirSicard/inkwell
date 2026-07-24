import Reveal from "./Reveal";
import SectionHeading from "./SectionHeading";

/*
  The four steps mirror the actual pipeline in the app
  (src-tauri/src/pipeline.rs): resample → VAD → transcribe → style → dictionary
  → snippets → optional polish → paste.
*/
const steps = [
  {
    n: "01",
    title: "Hold the hotkey",
    body: "Ctrl+Space out of the box, rebindable. Push-to-talk by default; toggle mode if you would rather press once and press again.",
  },
  {
    n: "02",
    title: "Speak",
    body: "Audio is captured from your chosen input and resampled to 16 kHz in memory. Nothing is uploaded, and nothing is written to disk unless you turn on the debug audio setting.",
  },
  {
    n: "03",
    title: "Let go",
    body: "Transcription starts on release and runs on your CPU through a local model. There is no live-as-you-speak preview — that does not exist yet.",
  },
  {
    n: "04",
    title: "The text appears",
    body: "Style formatting, your custom dictionary and your snippets are applied, then Inkwell writes to the clipboard and sends Cmd+V (Ctrl+V on Windows and Linux) to the focused app.",
  },
];

export default function HowItWorks() {
  return (
    <section
      id="how-it-works"
      aria-labelledby="how-it-works-title"
      className="mx-auto max-w-5xl scroll-mt-16 px-6 py-24 sm:py-28"
    >
      <SectionHeading
        eyebrow="The loop"
        id="how-it-works-title"
        title="One key, four steps, no round trip."
      />

      <ol className="grid gap-4 sm:grid-cols-2">
        {steps.map((step, i) => (
          <li key={step.n}>
            <Reveal delay={i * 70} className="h-full">
              <div className="surface h-full p-6">
                <span
                  className="font-mono text-xs tracking-widest"
                  style={{ color: "var(--accent)" }}
                >
                  {step.n}
                </span>
                <h3 className="mt-3 text-lg font-medium">{step.title}</h3>
                <p
                  className="mt-2 text-pretty text-sm leading-relaxed"
                  style={{ color: "var(--text-secondary)" }}
                >
                  {step.body}
                </p>
              </div>
            </Reveal>
          </li>
        ))}
      </ol>
    </section>
  );
}
