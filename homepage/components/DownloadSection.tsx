"use client";

import {
  APP_VERSION,
  GITHUB_RELEASES_LATEST_URL,
  GITHUB_RELEASES_URL,
} from "@/lib/constants";
import { usePlatform, type PlatformKey } from "@/lib/platform";
import DownloadButton from "./DownloadButton";
import Reveal from "./Reveal";
import SectionHeading from "./SectionHeading";
import { AppleIcon, LinuxIcon, WindowsIcon } from "./icons";

/*
  Per-platform facts come from .github/workflows/build.yml (targets built by CI:
  aarch64 + x86_64 macOS, x86_64 Windows, x86_64 Linux. The Linux job passes
  `--bundles deb,appimage`, so there is no .rpm to advertise) and from the app
  README. macOS Intel and Linux are `best_effort: true` in the matrix, i.e.
  continue-on-error, so a release can legitimately ship without them.
  Nothing here links to a guessed asset filename; the buttons point at the
  GitHub release page, which always lists what CI actually produced.
*/
const platforms: {
  key: PlatformKey;
  label: string;
  icon: React.ReactNode;
  tagline: string;
  artifacts: string;
  steps: string[];
}[] = [
  {
    key: "macos",
    label: "macOS",
    icon: <AppleIcon size={20} />,
    tagline: "Primary platform. Apple Silicon is the build that gets used daily.",
    artifacts: "Apple Silicon disk image; Intel is best-effort",
    steps: [
      "Drag Inkwell into Applications.",
      "Open it. The app and the disk image are both signed with a Developer ID and notarised by Apple, so there is no warning to click past.",
      "Allow Microphone access when asked.",
      "Allow Accessibility access. Inkwell pastes with a synthetic keystroke, which macOS blocks until you grant this. Without it you get a transcript and no text.",
    ],
  },
  {
    key: "windows",
    label: "Windows",
    icon: <WindowsIcon size={20} />,
    tagline: "Secondary platform, built by CI on every release.",
    artifacts: "NSIS installer (recommended) and MSI",
    steps: [
      "The installer is unsigned, so SmartScreen shows a warning.",
      "Click “More info”, then “Run anyway”.",
      "Per-app style rules match on the executable name here (outlook.exe, code.exe).",
    ],
  },
  {
    key: "linux",
    label: "Linux",
    icon: <LinuxIcon size={20} />,
    tagline: "Best effort. Built by CI, not regularly tested.",
    artifacts: "AppImage and .deb (x86_64)",
    steps: [
      "Expect rough edges; bug reports with your distro and desktop are welcome.",
      "Global hotkey and paste rely on the usual X11/Wayland input plumbing.",
      "Per-app style overrides stay off: the foreground app cannot be read here.",
    ],
  },
];

export default function DownloadSection() {
  const detected = usePlatform();

  return (
    <section
      id="download"
      aria-labelledby="download-title"
      className="mx-auto max-w-5xl scroll-mt-16 px-6 py-24 sm:py-28"
    >
      <SectionHeading
        eyebrow="Download"
        id="download-title"
        title={`Inkwell ${APP_VERSION}, free, no account.`}
        intro="Builds for every platform live on the GitHub releases page. Pick your file there. Nothing here asks for an email address."
      />

      <Reveal className="mb-10 flex flex-col items-center gap-4">
        <div className="flex flex-wrap items-center justify-center gap-3">
          <DownloadButton />
          <a href={GITHUB_RELEASES_URL} className="btn btn-secondary">
            All releases
          </a>
        </div>
        <p className="text-sm" style={{ color: "var(--text-tertiary)" }}>
          Opens the latest release on GitHub.
        </p>
      </Reveal>

      <div className="grid gap-4 md:grid-cols-3">
        {platforms.map((p, i) => {
          const isDetected = detected === p.key;
          return (
            <Reveal key={p.key} delay={i * 70} className="h-full">
              <div
                className="surface h-full p-6"
                style={
                  isDetected
                    ? {
                        borderColor: "var(--border-accent)",
                        background: "var(--bg-hover)",
                      }
                    : undefined
                }
              >
                <div className="flex items-center gap-3">
                  <span style={{ color: "var(--accent)" }}>{p.icon}</span>
                  <h3 className="text-base font-medium">{p.label}</h3>
                  {isDetected && (
                    <span
                      className="ml-auto rounded-full px-2 py-0.5 font-mono text-[0.625rem] uppercase tracking-wider"
                      style={{
                        background: "var(--accent-soft)",
                        color: "var(--accent)",
                      }}
                    >
                      You
                    </span>
                  )}
                </div>

                <p
                  className="mt-3 text-sm leading-relaxed"
                  style={{ color: "var(--text-secondary)" }}
                >
                  {p.tagline}
                </p>

                <p
                  className="mt-3 font-mono text-[0.75rem] leading-relaxed"
                  style={{ color: "var(--text-tertiary)" }}
                >
                  {p.artifacts}
                </p>

                {/* Ordered: these are first-launch steps in sequence, and the
                    macOS run (install, open, then the two permissions) only
                    makes sense read in order. */}
                <ol className="mt-4 space-y-2">
                  {p.steps.map((s) => (
                    <li
                      key={s}
                      className="text-sm leading-relaxed"
                      style={{ color: "var(--text-tertiary)" }}
                    >
                      {s}
                    </li>
                  ))}
                </ol>

                <a
                  href={GITHUB_RELEASES_LATEST_URL}
                  className="mt-5 inline-block text-sm underline decoration-dotted underline-offset-4"
                  style={{ color: "var(--text-primary)" }}
                >
                  Get the {p.label} build
                </a>
              </div>
            </Reveal>
          );
        })}
      </div>

      <Reveal className="mt-6">
        <div className="surface-quiet p-6">
          <h3 className="text-sm font-medium">
            Why your OS warns you about this app
          </h3>
          <p
            className="mt-2 text-pretty text-sm leading-relaxed"
            style={{ color: "var(--text-secondary)" }}
          >
            Nothing is code signed yet. An Apple Developer ID with notarisation
            and a Windows certificate both cost money every year, and Inkwell has
            no revenue. So macOS and Windows treat the builds as unknown
            software and say so loudly. That is the trade for a free app, not a
            sign that something is wrong. The source and the build workflow that
            produced these files are public, and you can build it yourself.
          </p>
        </div>
      </Reveal>

      <Reveal className="mt-6">
        <p
          className="text-center text-sm leading-relaxed"
          style={{ color: "var(--text-tertiary)" }}
        >
          The installer contains no speech models. First run offers Parakeet V3
          (670 MB) in one click; the rest of the catalogue lives in the Models
          tab, which appears once you turn on Advanced Mode. After that one
          download, dictation works with no connection at all.
        </p>
      </Reveal>
    </section>
  );
}
