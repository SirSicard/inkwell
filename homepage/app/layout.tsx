import type { Metadata, Viewport } from "next";
import { Geist, Geist_Mono } from "next/font/google";
import { SITE_URL } from "@/lib/constants";
import "./globals.css";

const geistSans = Geist({
  variable: "--font-geist-sans",
  subsets: ["latin"],
  display: "swap",
});

const geistMono = Geist_Mono({
  variable: "--font-geist-mono",
  subsets: ["latin"],
  display: "swap",
});

const title = "Inkwell — local-first dictation for your desktop";
const description =
  "Hold a key, speak, let go: your words land in whatever app you were typing in. Speech recognition runs on your own machine. Free, open source, no account, no telemetry.";

export const metadata: Metadata = {
  // SITE_URL is a placeholder in lib/constants.ts — swap it for the real domain
  // and the canonical URL plus the absolute OG image URL follow automatically.
  metadataBase: new URL(SITE_URL),
  title: {
    default: title,
    template: "%s — Inkwell",
  },
  description,
  applicationName: "Inkwell",
  keywords: [
    "dictation",
    "speech to text",
    "local first",
    "offline transcription",
    "open source",
    "macOS",
    "Windows",
    "privacy",
  ],
  alternates: {
    canonical: "/",
  },
  openGraph: {
    type: "website",
    siteName: "Inkwell",
    title,
    description,
    url: "/",
    locale: "en_GB",
    // public/og.png is a real 1200x630 raster: charcoal field, cream ink blob,
    // the wordmark. Rendered from SVG, not from the WebGL canvas — a crawler
    // never runs the shader, so the share card has to be a flat file.
    images: [
      {
        url: "/og.png",
        width: 1200,
        height: 630,
        alt: "Inkwell — local-first dictation for your desktop",
      },
    ],
  },
  twitter: {
    card: "summary_large_image",
    title,
    description,
    images: ["/og.png"],
  },
  robots: {
    index: true,
    follow: true,
  },
};

export const viewport: Viewport = {
  themeColor: "#0e0e11",
  colorScheme: "dark",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    // suppressHydrationWarning is required, not cosmetic: the inline script
    // below adds `js` to <html> before React hydrates, so the className React
    // rendered on the server no longer matches the live DOM. Without this,
    // every visitor's console gets a hydration-mismatch error on load.
    // The suppression is one level deep and does not affect any child.
    <html
      lang="en"
      suppressHydrationWarning
      className={`${geistSans.variable} ${geistMono.variable}`}
    >
      <body className="antialiased">
        {/*
          Marks the document as script-capable before the page paints, so the
          scroll-reveal styles in globals.css can start hidden without a no-JS
          visitor ever losing content.
        */}
        <script
          dangerouslySetInnerHTML={{
            __html: "document.documentElement.classList.add('js')",
          }}
        />
        {children}
      </body>
    </html>
  );
}
