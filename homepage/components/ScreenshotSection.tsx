import Image from "next/image";
import Reveal from "./Reveal";
import SectionHeading from "./SectionHeading";

/*
  Captured from 0.2.6 against a throwaway home directory with invented
  transcripts, not a real dictation history. Regenerate the same way rather than
  screenshotting a working install: the dashboard renders whatever is in the
  local SQLite file, and that is somebody's actual writing.

  Intrinsic sizes are the real pixel dimensions of the files in public/shots, so
  next/image reserves the right box and the page does not shift as they load.
*/
const shots = [
  {
    src: "/shots/inkwell-dashboard.png",
    alt: "Inkwell's dashboard, listing recent transcripts with the time, duration and model used for each",
    title: "Everything you dictated, searchable",
    body: "History lives in a SQLite file on your machine. Search it, edit it, export it to TXT, SRT, JSON or CSV. Nothing syncs anywhere.",
  },
  {
    src: "/shots/inkwell-modes.png",
    alt: "The Modes tab, with a default mode plus Casual and Relaxed modes bound to lists of applications",
    title: "Writes differently depending on where you are",
    body: "A mode bundles a style, speech cleanup and AI polish, and switches itself on based on the app you are typing into. Formal in email, lowercase in Slack, without touching a setting.",
  },
  {
    src: "/shots/inkwell-ai.png",
    alt: "The AI tab, showing the API key field with Groq selected and labelled as a free key",
    title: "Optional, and your own key",
    body: "AI polish and voice editing are the only features that use the internet, and only if you switch them on. Groq's free tier covers ordinary use. The key is kept in your system keyring.",
  },
];

export default function ScreenshotSection() {
  return (
    <section
      id="screenshots"
      aria-labelledby="screenshots-title"
      className="mx-auto max-w-5xl scroll-mt-16 px-6 py-24 sm:py-28"
    >
      <SectionHeading
        eyebrow="A look inside"
        id="screenshots-title"
        title="What you actually get."
        intro="Three of the screens you will spend time in. No mockups; this is the app running."
      />

      <div className="mt-4 space-y-16">
        {shots.map((s) => (
          <Reveal key={s.src}>
            <figure>
              <div
                className="surface overflow-hidden"
                style={{ padding: 0, lineHeight: 0 }}
              >
                <Image
                  src={s.src}
                  alt={s.alt}
                  width={1600}
                  height={1038}
                  sizes="(max-width: 64rem) 100vw, 64rem"
                  className="h-auto w-full"
                />
              </div>
              <figcaption className="mt-4">
                <p className="font-medium">{s.title}</p>
                <p
                  className="mt-1 max-w-2xl text-sm leading-relaxed"
                  style={{ color: "var(--text-secondary)" }}
                >
                  {s.body}
                </p>
              </figcaption>
            </figure>
          </Reveal>
        ))}
      </div>
    </section>
  );
}
