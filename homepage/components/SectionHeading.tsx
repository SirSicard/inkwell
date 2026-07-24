import type { ReactNode } from "react";
import Reveal from "./Reveal";

export default function SectionHeading({
  eyebrow,
  title,
  id,
  intro,
}: {
  eyebrow: string;
  title: string;
  id: string;
  intro?: ReactNode;
}) {
  return (
    <Reveal className="mx-auto mb-12 max-w-2xl text-center sm:mb-16">
      <p className="eyebrow">{eyebrow}</p>
      <h2
        id={id}
        className="mt-3 text-balance text-[clamp(1.75rem,4vw,2.5rem)] font-semibold leading-tight tracking-tight"
      >
        {title}
      </h2>
      {intro ? (
        <p
          className="mt-4 text-pretty text-base leading-relaxed"
          style={{ color: "var(--text-secondary)" }}
        >
          {intro}
        </p>
      ) : null}
    </Reveal>
  );
}
