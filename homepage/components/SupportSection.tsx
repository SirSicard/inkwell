import {
  DONATION_SUGGESTED,
  DONATION_URL,
  GITHUB_ISSUES_URL,
  GITHUB_URL,
} from "@/lib/constants";
import Reveal from "./Reveal";
import { HeartIcon } from "./icons";

export default function SupportSection() {
  return (
    <section
      id="support"
      aria-labelledby="support-title"
      className="mx-auto max-w-3xl scroll-mt-16 px-6 py-24 sm:py-28"
    >
      <Reveal>
        <div className="surface p-8 text-center sm:p-12">
          <span
            className="inline-flex h-11 w-11 items-center justify-center rounded-full"
            style={{ background: "var(--accent-soft)", color: "var(--accent)" }}
          >
            <HeartIcon size={20} />
          </span>

          <h2
            id="support-title"
            className="mt-5 text-[clamp(1.5rem,3.5vw,2rem)] font-semibold tracking-tight"
          >
            Free forever. Tips optional.
          </h2>

          <p
            className="mx-auto mt-4 max-w-xl text-pretty text-base leading-relaxed"
            style={{ color: "var(--text-secondary)" }}
          >
            There is no paid tier, no licence key and no feature waiting behind a
            paywall, and there never will be. If Inkwell saves you time, you can
            buy me a coffee. {DONATION_SUGGESTED} is the suggested amount and it
            changes nothing about the app.
          </p>

          <div className="mt-8 flex flex-wrap items-center justify-center gap-3">
            <a
              href={DONATION_URL}
              className="btn btn-primary"
              rel="noopener noreferrer"
              target="_blank"
            >
              Buy me a coffee
            </a>
            <a href={GITHUB_ISSUES_URL} className="btn btn-secondary">
              Report a bug instead
            </a>
          </div>

          <p
            className="mx-auto mt-6 max-w-lg text-sm leading-relaxed"
            style={{ color: "var(--text-tertiary)" }}
          >
            Honestly, the unpaid help is worth more: tell me what broke on your
            hardware, or send a pull request to{" "}
            <a
              href={GITHUB_URL}
              className="underline decoration-dotted underline-offset-4"
              style={{ color: "var(--text-secondary)" }}
            >
              the repo
            </a>
            .
          </p>
        </div>
      </Reveal>
    </section>
  );
}
