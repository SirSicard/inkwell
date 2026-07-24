"use client";

import { useEffect, useRef, type ReactNode } from "react";

/**
 * Fade/slide content in as it scrolls into view.
 *
 * Progressive enhancement only: the markup ships visible, `.js` on <html>
 * (set by the inline script in app/layout.tsx) is what opts it into the hidden
 * start state, and the whole transition is wrapped in a
 * `prefers-reduced-motion: no-preference` query in globals.css. With reduced
 * motion the element simply renders in place.
 */
export default function Reveal({
  children,
  className = "",
  delay = 0,
}: {
  children: ReactNode;
  className?: string;
  delay?: number;
}) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;

    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      el.classList.add("is-visible");
      return;
    }

    const io = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          el.classList.add("is-visible");
          io.disconnect();
        }
      },
      { rootMargin: "0px 0px -10% 0px", threshold: 0.05 },
    );
    io.observe(el);
    return () => io.disconnect();
  }, []);

  return (
    <div
      ref={ref}
      className={`reveal ${className}`}
      style={delay ? { transitionDelay: `${delay}ms` } : undefined}
    >
      {children}
    </div>
  );
}
