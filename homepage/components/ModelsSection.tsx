import { SHERPA_ONNX_URL } from "@/lib/constants";
import Reveal from "./Reveal";
import SectionHeading from "./SectionHeading";

/*
  The whole catalogue, matching MODELS in src-tauri/src/models.rs. It is five
  entries, not the thirteen this table used to list: the rest lost on every axis
  at once and were cut in 0.2.2, so a visitor was being sold a choice the app no
  longer offers.

  Word error rates are measured, not quoted from a leaderboard. Source is
  docs/qwen3-spike-2026-07-31.md, eight recordings of one voice scored with
  src-tauri/examples/ab_models.rs. Re-measure before editing a number here, and
  keep it in step with docs/media/models-*.svg, which is generated from the same
  figures by scripts/gen-model-chart.py.
*/
const models = [
  {
    name: "Qwen3 ASR",
    vendor: "Alibaba",
    size: "940 MB",
    wer: "5.6%",
    languages: "30, incl. Nordic",
    note: "Most accurate, and the only one covering English and Nordic together.",
  },
  {
    name: "Parakeet V3",
    vendor: "NVIDIA",
    size: "670 MB",
    wer: "10.5%",
    languages: "25 European",
    note: "The default. Detects the language for you.",
  },
  {
    name: "Parakeet V2",
    vendor: "NVIDIA",
    size: "670 MB",
    wer: "8.0%",
    languages: "English",
    note: "Same download as V3, more accurate if you only speak English.",
  },
  {
    name: "SenseVoice",
    vendor: "Alibaba",
    size: "240 MB",
    wer: "9.3%",
    languages: "en, zh, ja, ko, yue",
    note: "A quarter of the size, and the fastest here.",
  },
  {
    name: "Whisper Turbo",
    vendor: "OpenAI",
    size: "800 MB",
    wer: "9.3%",
    languages: "99",
    note: "For the languages the others do not reach.",
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
            Five speech-to-text models download from inside the app, all running
            locally through{" "}
            <a
              href={SHERPA_ONNX_URL}
              className="underline decoration-dotted underline-offset-4"
              style={{ color: "var(--text-primary)" }}
            >
              sherpa-onnx
            </a>
            . Every one is here for a stated reason; the list was three times
            longer until the ones that lost on every axis were cut. Nothing
            ships inside the installer. The files come from Hugging Face when
            you pick one, and after that it needs no internet at all.
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
                  Errors
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
                    className="whitespace-nowrap px-5 py-4 align-top font-mono text-[0.8125rem]"
                    style={{ color: "var(--text-secondary)" }}
                  >
                    {m.wer}
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

      <Reveal>
        <p
          className="mt-4 text-sm"
          style={{ color: "var(--text-tertiary)" }}
        >
          Error rates are measured rather than quoted: eight recordings of one
          voice, scored against what was actually said. Directional, not a
          benchmark, and your voice is not that voice.
        </p>
      </Reveal>
    </section>
  );
}
