import { APP_VERSION, GITHUB_URL } from "@/lib/constants";
import DownloadButton from "./DownloadButton";
import InkCanvas from "./InkCanvas";
import { GitHubIcon } from "./icons";

export default function Hero() {
  return (
    <section
      aria-labelledby="hero-title"
      className="relative isolate overflow-hidden"
    >
      {/* The ink as a full-bleed backdrop rather than a framed panel.
          Three layers, in order:
            1. the shader, in its charcoal-on-charcoal "backdrop" palette
            2. a centre scrim, so the copy sits on near-solid ground instead of
               on whatever the blob happens to be doing behind that word
            3. top and bottom fades to the page colour, so the hero dissolves
               into the sticky header above and the next section below instead
               of ending on a hard edge
          All of it is aria-hidden and pointer-events-none: it is texture, and it
          must never intercept a click meant for the download button. */}
      <div className="pointer-events-none absolute inset-0 -z-10" aria-hidden="true">
        <InkCanvas variant="backdrop" />
        <div
          className="absolute inset-0"
          style={{
            // Deliberately light. Measured contrast of the body text over the
            // densest part of the ink is 14.4:1, so the scrim is not carrying
            // legibility — the palette already does. Its only job is to take the
            // edge off the grain and motion directly behind the words. An
            // opaque centre scrim also cancels the blob exactly where the blob
            // is strongest, since both are centred, which leaves the ink
            // visible only at the corners.
            background:
              "radial-gradient(62% 55% at 50% 46%, rgba(14,14,17,0.42) 0%, rgba(14,14,17,0.24) 45%, rgba(14,14,17,0.08) 75%, rgba(14,14,17,0) 100%)",
          }}
        />
        <div
          className="absolute inset-x-0 top-0 h-28"
          style={{
            background:
              "linear-gradient(to bottom, var(--bg-base) 0%, rgba(14,14,17,0) 100%)",
          }}
        />
        <div
          className="absolute inset-x-0 bottom-0 h-40"
          style={{
            background:
              "linear-gradient(to top, var(--bg-base) 0%, rgba(14,14,17,0) 100%)",
          }}
        />
      </div>

      <div className="mx-auto flex max-w-3xl flex-col items-center px-6 pb-28 pt-24 text-center sm:pt-32 lg:pb-36 lg:pt-40">
        {/* Secondary rather than tertiary, and measured: --text-tertiary is 58%
            alpha, which over the brightest part of the ink lands at 4.43:1 —
            under the 4.5:1 AA floor for body text. The rest of the page can
            afford tertiary because it sits on the flat page colour; inside the
            hero the same token does not clear the bar. Underlining still
            distinguishes the link below, so no hierarchy is lost. */}
        <p
          className="font-mono text-[0.6875rem] uppercase tracking-[0.22em]"
          style={{ color: "var(--text-secondary)" }}
        >
          Inkwell v{APP_VERSION} · MIT licensed · free forever
        </p>

        {/* The h1 carries the proposition, not the wordmark. "Inkwell" is
            already in the sticky header and in the meta line directly overhead;
            spending the largest type on the page repeating it would leave the
            loudest thing on the page saying nothing. */}
        <h1
          id="hero-title"
          className="mt-6 text-balance text-[clamp(2.5rem,6.4vw,4.5rem)] font-semibold leading-[1.02] tracking-tight"
        >
          Hold a key, speak, let go.
        </h1>

        <p className="mt-6 max-w-xl text-pretty text-[clamp(1.125rem,2.2vw,1.375rem)] leading-snug">
          The text lands in whatever app you were already typing in.
        </p>

        <p
          className="mt-5 max-w-xl text-pretty text-base leading-relaxed"
          style={{ color: "var(--text-secondary)" }}
        >
          Speech recognition runs on your own machine, so your voice never
          leaves it. Free forever, open source, no account, no telemetry.
        </p>

        <div className="mt-10 flex flex-wrap items-center justify-center gap-3">
          <DownloadButton />
          <a href={GITHUB_URL} className="btn btn-secondary">
            <GitHubIcon size={17} />
            Source on GitHub
          </a>
        </div>

        <p
          className="mt-6 max-w-md text-sm leading-relaxed"
          style={{ color: "var(--text-secondary)" }}
        >
          Builds are not code signed yet — first launch needs one extra step.{" "}
          <a
            href="#download"
            className="underline decoration-dotted underline-offset-4"
            style={{ color: "var(--text-secondary)" }}
          >
            How to open it
          </a>
          .
        </p>
      </div>
    </section>
  );
}
