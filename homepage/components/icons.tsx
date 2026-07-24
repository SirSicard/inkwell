/* Inline SVG icons. Decorative unless a title is passed, so they are hidden
   from assistive tech by default and the adjacent text carries the meaning. */

type IconProps = { size?: number; className?: string };

const base = (size: number) => ({
  width: size,
  height: size,
  viewBox: "0 0 24 24",
  "aria-hidden": true as const,
  focusable: "false" as const,
});

export function AppleIcon({ size = 18, className }: IconProps) {
  return (
    <svg {...base(size)} className={className} fill="currentColor">
      <path d="M12.152 6.896c-.948 0-2.415-1.078-3.96-1.04-2.04.027-3.91 1.183-4.961 3.014-2.117 3.675-.546 9.103 1.519 12.09 1.013 1.454 2.208 3.09 3.792 3.039 1.52-.065 2.09-.987 3.935-.987 1.831 0 2.35.987 3.96.948 1.637-.026 2.676-1.48 3.676-2.948 1.156-1.688 1.636-3.325 1.662-3.415-.039-.013-3.182-1.221-3.22-4.857-.026-3.04 2.48-4.494 2.597-4.559-1.429-2.09-3.623-2.324-4.39-2.376-2-.156-3.675 1.09-4.61 1.09zM15.53 3.83c.843-1.012 1.4-2.427 1.245-3.83-1.207.052-2.662.805-3.532 1.818-.78.896-1.454 2.338-1.273 3.714 1.338.104 2.715-.688 3.559-1.701" />
    </svg>
  );
}

export function WindowsIcon({ size = 18, className }: IconProps) {
  return (
    <svg {...base(size)} className={className} fill="currentColor">
      <path d="M0 3.449L9.75 2.1v9.451H0m10.949-9.602L24 0v11.551H10.949M0 12.6h9.75v9.451L0 20.699M10.949 12.749H24V24l-12.9-1.801" />
    </svg>
  );
}

export function LinuxIcon({ size = 18, className }: IconProps) {
  return (
    <svg
      {...base(size)}
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M9 3.5c0-1.1.9-2 2-2h2c1.1 0 2 .9 2 2v3.2c0 1 .3 2 .9 2.8l2 2.6c1.4 1.9 2.1 4.2 2.1 6.5 0 1.3-1 2.4-2.3 2.4H6.3C5 21 4 19.9 4 18.6c0-2.3.7-4.6 2.1-6.5l2-2.6c.6-.8.9-1.8.9-2.8z" />
      <path d="M10 6.5h.01M14 6.5h.01M10.5 10.5c.9.7 2.1.7 3 0" />
    </svg>
  );
}

export function GitHubIcon({ size = 18, className }: IconProps) {
  return (
    <svg {...base(size)} className={className} fill="currentColor">
      <path d="M12 .5C5.73.5.5 5.73.5 12a11.5 11.5 0 0 0 7.86 10.92c.58.1.79-.25.79-.56v-2.1c-3.2.7-3.88-1.37-3.88-1.37-.53-1.34-1.29-1.7-1.29-1.7-1.05-.72.08-.7.08-.7 1.16.08 1.77 1.2 1.77 1.2 1.03 1.77 2.7 1.26 3.36.96.1-.75.4-1.26.73-1.55-2.55-.29-5.24-1.28-5.24-5.7 0-1.26.45-2.29 1.19-3.1-.12-.29-.52-1.46.11-3.05 0 0 .97-.31 3.18 1.18a11 11 0 0 1 5.8 0c2.2-1.49 3.17-1.18 3.17-1.18.63 1.59.24 2.76.12 3.05.74.81 1.18 1.84 1.18 3.1 0 4.43-2.69 5.4-5.25 5.69.41.36.78 1.06.78 2.14v3.17c0 .31.2.67.8.56A11.5 11.5 0 0 0 23.5 12C23.5 5.73 18.27.5 12 .5z" />
    </svg>
  );
}

export function DownloadIcon({ size = 18, className }: IconProps) {
  return (
    <svg
      {...base(size)}
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M12 3v12m0 0-4.5-4.5M12 15l4.5-4.5M4 20h16" />
    </svg>
  );
}

export function LockIcon({ size = 20, className }: IconProps) {
  return (
    <svg
      {...base(size)}
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <rect x="3.5" y="10.5" width="17" height="11" rx="2" />
      <path d="M7.5 10.5V7a4.5 4.5 0 0 1 9 0v3.5" />
    </svg>
  );
}

export function WaveIcon({ size = 20, className }: IconProps) {
  return (
    <svg
      {...base(size)}
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
    >
      <path d="M3 12h1.5M7 8v8M10.5 5v14M14 8.5v7M17.5 10.5v3M21 12h-.5" />
    </svg>
  );
}

export function PenIcon({ size = 20, className }: IconProps) {
  return (
    <svg
      {...base(size)}
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M12 20h9" />
      <path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4z" />
    </svg>
  );
}

export function KeyboardIcon({ size = 20, className }: IconProps) {
  return (
    <svg
      {...base(size)}
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <rect x="2.5" y="6" width="19" height="12" rx="2" />
      <path d="M6.5 9.5h.01M10 9.5h.01M13.5 9.5h.01M17 9.5h.01M7.5 14h9" />
    </svg>
  );
}

export function FileIcon({ size = 20, className }: IconProps) {
  return (
    <svg
      {...base(size)}
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" />
      <path d="M14 3v5h5M9 13h6M9 17h4" />
    </svg>
  );
}

export function TrayIcon({ size = 20, className }: IconProps) {
  return (
    <svg
      {...base(size)}
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <rect x="2.5" y="4.5" width="19" height="15" rx="2" />
      <path d="M2.5 14.5h5l1.5 2.5h6l1.5-2.5h5" />
    </svg>
  );
}

export function HeartIcon({ size = 20, className }: IconProps) {
  return (
    <svg
      {...base(size)}
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M12 20.3 4.6 13a4.6 4.6 0 1 1 6.5-6.5l.9.9.9-.9A4.6 4.6 0 1 1 19.4 13z" />
    </svg>
  );
}
