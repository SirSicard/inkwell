import { APP_VERSION, GITHUB_URL } from "@/lib/constants";
import DownloadButton from "./DownloadButton";
import InkCanvas from "./InkCanvas";
import { GitHubIcon } from "./icons";

export default function Hero() {
  return (
    <section
      aria-labelledby="hero-title"
      className="relative mx-auto grid max-w-6xl items-center gap-14 px-6 pb-20 pt-20 sm:pt-28 lg:grid-cols-[1.05fr_0.95fr] lg:gap-16 lg:pb-28"
    >
      <div>
        <p
          className="font-mono text-[0.6875rem] uppercase tracking-[0.22em]"
          style={{ color: "var(--text-tertiary)" }}
        >
          Inkwell v{APP_VERSION} · MIT licensed · free forever
        </p>

        {/* The h1 carries the proposition, not the wordmark. "Inkwell" is
            already in the sticky header two rows above and in the meta line
            directly overhead; spending the largest type on the page repeating
            it would leave the loudest thing on the page saying nothing. The
            brand is the ink, and the ink is the panel. */}
        <h1
          id="hero-title"
          className="mt-5 max-w-2xl text-balance text-[clamp(2.25rem,5.2vw,3.75rem)] font-semibold leading-[1.02] tracking-tight"
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

        <div className="mt-9 flex flex-wrap items-center gap-3">
          <DownloadButton />
          <a href={GITHUB_URL} className="btn btn-secondary">
            <GitHubIcon size={17} />
            Source on GitHub
          </a>
        </div>

        <p
          className="mt-5 max-w-md text-sm leading-relaxed"
          style={{ color: "var(--text-tertiary)" }}
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

      {/* The ink panel: the app's cream shader framed by the charcoal page,
          the same inversion the desktop UI uses. */}
      <div className="relative">
        <div
          className="relative aspect-[4/3] w-full overflow-hidden rounded-2xl sm:aspect-[16/11]"
          style={{ border: "1px solid var(--border-strong)" }}
        >
          <InkCanvas />
        </div>
        <p
          className="mt-3 text-center font-mono text-[0.6875rem] tracking-wide"
          style={{ color: "var(--text-tertiary)" }}
        >
          The ink reacts to your voice inside the app.
        </p>
      </div>
    </section>
  );
}
