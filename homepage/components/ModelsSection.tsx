import { SHERPA_ONNX_URL } from "@/lib/constants";
import Reveal from "./Reveal";
import SectionHeading from "./SectionHeading";

/*
  A curated slice of the in-app catalogue, copied verbatim from
  src/tabs/ModelsTab.tsx (MODEL_CATALOG). Sizes and language counts are the ones
  the app itself shows.

  The catalogue lists 13 entries but only 12 are obtainable: the `hf_base` match
  in src-tauri/src/commands.rs has no URL for "moonshine-tiny", so pressing
  Download on it returns Err("No download URL for: moonshine-tiny"). It is
  therefore deliberately absent from this table, and Whisper Tiny (98 MB, and
  genuinely downloadable) stands in as the small end of the range.
*/
const models = [
  {
    name: "Parakeet V3",
    vendor: "NVIDIA",
    size: "670 MB",
    languages: "25 European",
    note: "Default. Best accuracy-to-speed balance.",
  },
  {
    name: "Whisper Turbo",
    vendor: "OpenAI",
    size: "800 MB",
    languages: "99",
    note: "Balanced multilingual all-rounder.",
  },
  {
    name: "Whisper Large V3",
    vendor: "OpenAI",
    size: "1.5 GB",
    languages: "99",
    note: "Best multilingual accuracy, slowest.",
  },
  {
    name: "Moonshine Base",
    vendor: "Useful Sensors",
    size: "288 MB",
    languages: "English",
    note: "Fast, modest download.",
  },
  {
    name: "SenseVoice",
    vendor: "Alibaba",
    size: "160 MB",
    languages: "5",
    note: "Chinese, English, Japanese, Korean, Cantonese.",
  },
  {
    name: "Whisper Tiny",
    vendor: "OpenAI",
    size: "98 MB",
    languages: "99",
    note: "Smallest download, lowest accuracy.",
  },
];

export default function ModelsSection() {
  return (
    <section
      id="models"
      aria-labelledby="models-title"
      className="mx-auto max-w-5xl scroll-mt-16 px-6 py-24 sm:py-28"
    >
      <SectionHeading
        eyebrow="Models"
        id="models-title"
        title="Pick the trade-off you want."
        intro={
          <>
            Twelve speech-to-text models download from inside the app, all
            running locally through{" "}
            <a
              href={SHERPA_ONNX_URL}
              className="underline decoration-dotted underline-offset-4"
              style={{ color: "var(--text-primary)" }}
            >
              sherpa-onnx
            </a>
            . Six are below; the other six are five more Whisper sizes and an
            English-tuned Parakeet V2. Nothing ships inside the installer. The
            files come from Hugging Face when you pick one, and after that
            download it needs no internet at all.
          </>
        }
      />

      <Reveal>
        <div className="surface overflow-x-auto">
          <table className="w-full min-w-[36rem] border-collapse text-left text-sm">
            <caption className="sr-only">
              Selected speech-to-text models available in Inkwell, with download
              size and language coverage
            </caption>
            <thead>
              <tr style={{ color: "var(--text-tertiary)" }}>
                <th scope="col" className="px-5 py-3 font-mono text-[0.6875rem] uppercase tracking-widest">
                  Model
                </th>
                <th scope="col" className="px-5 py-3 font-mono text-[0.6875rem] uppercase tracking-widest">
                  Size
                </th>
                <th scope="col" className="px-5 py-3 font-mono text-[0.6875rem] uppercase tracking-widest">
                  Languages
                </th>
                <th scope="col" className="px-5 py-3 font-mono text-[0.6875rem] uppercase tracking-widest">
                  Notes
                </th>
              </tr>
            </thead>
            <tbody>
              {models.map((m) => (
                <tr key={m.name} style={{ borderTop: "1px solid var(--border)" }}>
                  <th scope="row" className="px-5 py-4 align-top font-medium">
                    {m.name}
                    <span
                      className="mt-0.5 block font-mono text-[0.6875rem] font-normal uppercase tracking-wider"
                      style={{ color: "var(--text-tertiary)" }}
                    >
                      {m.vendor}
                    </span>
                  </th>
                  <td
                    className="whitespace-nowrap px-5 py-4 align-top font-mono text-[0.8125rem]"
                    style={{ color: "var(--text-secondary)" }}
                  >
                    {m.size}
                  </td>
                  <td
                    className="whitespace-nowrap px-5 py-4 align-top"
                    style={{ color: "var(--text-secondary)" }}
                  >
                    {m.languages}
                  </td>
                  <td
                    className="px-5 py-4 align-top"
                    style={{ color: "var(--text-secondary)" }}
                  >
                    {m.note}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </Reveal>
    </section>
  );
}
