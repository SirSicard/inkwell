import {
  APP_VERSION,
  AUTHOR_NAME,
  AUTHOR_URL,
  GITHUB_ISSUES_URL,
  GITHUB_LICENSE_URL,
  GITHUB_RELEASES_URL,
  GITHUB_URL,
  HANDY_URL,
  SHERPA_ONNX_URL,
  SILERO_VAD_URL,
  TAURI_URL,
} from "@/lib/constants";
import { GitHubIcon } from "./icons";

const links = [
  { label: "GitHub", href: GITHUB_URL },
  { label: "Releases", href: GITHUB_RELEASES_URL },
  { label: "Issues", href: GITHUB_ISSUES_URL },
  { label: "MIT licence", href: GITHUB_LICENSE_URL },
  { label: "Privacy", href: "#privacy" },
];

export default function SiteFooter() {
  return (
    <footer
      className="mt-8 border-t"
      style={{ borderColor: "var(--border)" }}
      aria-labelledby="footer-title"
    >
      <h2 id="footer-title" className="sr-only">
        Site footer
      </h2>
      <div className="mx-auto flex max-w-6xl flex-col gap-8 px-6 py-12 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <p className="text-sm font-medium tracking-tight">Inkwell</p>
          <p
            className="mt-2 max-w-sm text-sm leading-relaxed"
            style={{ color: "var(--text-tertiary)" }}
          >
            Local-first dictation for the desktop. Version {APP_VERSION}, MIT
            licensed, built by{" "}
            <a
              href={AUTHOR_URL}
              className="underline decoration-dotted underline-offset-4"
              style={{ color: "var(--text-secondary)" }}
            >
              {AUTHOR_NAME}
            </a>
            .
          </p>
          <p
            className="mt-3 max-w-sm text-sm leading-relaxed"
            style={{ color: "var(--text-tertiary)" }}
          >
            Originally based on{" "}
            <a
              href={HANDY_URL}
              className="underline decoration-dotted underline-offset-4"
            >
              Handy
            </a>{" "}
            by CJ Pais. Speech recognition by{" "}
            <a
              href={SHERPA_ONNX_URL}
              className="underline decoration-dotted underline-offset-4"
            >
              sherpa-onnx
            </a>
            , silence detection by{" "}
            <a
              href={SILERO_VAD_URL}
              className="underline decoration-dotted underline-offset-4"
            >
              Silero VAD
            </a>
            , desktop shell by{" "}
            <a
              href={TAURI_URL}
              className="underline decoration-dotted underline-offset-4"
            >
              Tauri
            </a>
            .
          </p>
        </div>

        <nav aria-label="Footer">
          <ul className="flex flex-wrap gap-x-6 gap-y-3 text-sm sm:flex-col sm:items-end">
            {links.map((l) => (
              <li key={l.label}>
                <a
                  href={l.href}
                  className="inline-flex items-center gap-2 transition-colors hover:text-[var(--text-primary)]"
                  style={{ color: "var(--text-secondary)" }}
                >
                  {l.label === "GitHub" && <GitHubIcon size={15} />}
                  {l.label}
                </a>
              </li>
            ))}
          </ul>
        </nav>
      </div>
    </footer>
  );
}
