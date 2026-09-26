import {
  MAINTAINER_GITHUB_URL,
  SIGNPATH_FOUNDATION_URL,
  SIGNPATH_URL,
} from "@/lib/constants";
import Reveal from "./Reveal";
import SectionHeading from "./SectionHeading";

/*
  The SignPath Foundation programme requires this policy on the project
  homepage, in this wording (signpath.org/terms). The README carries the same
  text. Keep the status line true: remove it once a signed release ships.
*/

const roles = [
  { role: "Committers and reviewers", who: "SirSicard" },
  { role: "Approvers", who: "SirSicard" },
];

export default function CodeSigningSection() {
  return (
    <section
      id="code-signing"
      aria-labelledby="code-signing-title"
      className="mx-auto max-w-3xl scroll-mt-16 px-6 py-24 sm:py-28"
    >
      <SectionHeading
        eyebrow="Code signing"
        id="code-signing-title"
        title="Code signing policy."
      />

      <Reveal>
        <div className="surface space-y-4 p-6 text-sm leading-relaxed sm:p-7">
          <p>
            Free code signing provided by{" "}
            <a
              href={SIGNPATH_URL}
              className="underline decoration-dotted underline-offset-4"
            >
              SignPath.io
            </a>
            , certificate by{" "}
            <a
              href={SIGNPATH_FOUNDATION_URL}
              className="underline decoration-dotted underline-offset-4"
            >
              SignPath Foundation
            </a>
            .
          </p>
          <ul className="space-y-1" style={{ color: "var(--text-secondary)" }}>
            {roles.map((r) => (
              <li key={r.role}>
                {r.role}:{" "}
                <a
                  href={MAINTAINER_GITHUB_URL}
                  className="underline decoration-dotted underline-offset-4"
                >
                  {r.who}
                </a>
              </li>
            ))}
          </ul>
          <p style={{ color: "var(--text-secondary)" }}>
            Only artifacts built by this repository&rsquo;s GitHub Actions
            workflows from this repository&rsquo;s own source are signed. Each
            release signing request is approved by hand.
          </p>
          <p style={{ color: "var(--text-secondary)" }}>
            <span className="font-medium" style={{ color: "var(--text-primary)" }}>
              Privacy policy:
            </span>{" "}
            see{" "}
            <a
              href="#privacy"
              className="underline decoration-dotted underline-offset-4"
            >
              Privacy
            </a>
            . This program does not send your audio anywhere. It contacts
            networked systems only to check for updates, to download the speech
            and voice-detection models it runs on, and, when you use AI polish
            or voice editing with your own API key, to send text to the
            provider you selected.
          </p>
          <p
            className="border-t pt-4"
            style={{
              borderColor: "var(--border)",
              color: "var(--text-tertiary)",
            }}
          >
            Status: Windows builds up to v0.2.9 are not signed yet.
          </p>
        </div>
      </Reveal>
    </section>
  );
}
