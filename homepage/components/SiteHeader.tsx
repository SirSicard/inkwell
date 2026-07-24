import { GITHUB_URL } from "@/lib/constants";
import { GitHubIcon } from "./icons";

const nav = [
  { label: "How it works", href: "#how-it-works" },
  { label: "Features", href: "#features" },
  { label: "Privacy", href: "#privacy" },
  { label: "Download", href: "#download" },
];

export default function SiteHeader() {
  return (
    <header
      className="sticky top-0 z-50 border-b"
      style={{
        borderColor: "var(--border)",
        background: "var(--bg-base)",
      }}
    >
      <div className="mx-auto flex max-w-6xl items-center gap-6 px-6 py-3.5">
        <a
          href="#top"
          className="text-sm font-semibold tracking-tight"
          style={{ color: "var(--text-primary)" }}
        >
          Inkwell
        </a>

        <nav aria-label="Primary" className="ml-auto hidden sm:block">
          <ul className="flex items-center gap-6 text-sm">
            {nav.map((item) => (
              <li key={item.href}>
                <a
                  href={item.href}
                  className="transition-colors hover:text-[var(--text-primary)]"
                  style={{ color: "var(--text-secondary)" }}
                >
                  {item.label}
                </a>
              </li>
            ))}
          </ul>
        </nav>

        <a
          href={GITHUB_URL}
          className="ml-auto inline-flex items-center gap-2 text-sm transition-colors hover:text-[var(--text-primary)] sm:ml-0"
          style={{ color: "var(--text-secondary)" }}
        >
          <GitHubIcon size={16} />
          <span className="sr-only sm:not-sr-only">GitHub</span>
        </a>
      </div>
    </header>
  );
}
