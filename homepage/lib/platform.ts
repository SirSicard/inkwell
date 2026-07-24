"use client";

import { useEffect, useState } from "react";

export type PlatformKey = "macos" | "windows" | "linux";

type UAData = { platform?: string };

/**
 * Best-effort OS detection. Returns null until the client has mounted so the
 * server-rendered markup and the first client render agree (no hydration
 * mismatch) and so the UI can show a platform-neutral label meanwhile.
 */
export function detectPlatform(): PlatformKey | null {
  if (typeof navigator === "undefined") return null;

  const uaData = (navigator as Navigator & { userAgentData?: UAData })
    .userAgentData;
  const hint = (uaData?.platform || navigator.userAgent || "").toLowerCase();

  if (hint.includes("mac") || hint.includes("iphone") || hint.includes("ipad")) {
    return "macos";
  }
  if (hint.includes("win")) return "windows";
  if (hint.includes("linux") || hint.includes("android")) return "linux";
  return null;
}

export function usePlatform(): PlatformKey | null {
  const [platform, setPlatform] = useState<PlatformKey | null>(null);
  useEffect(() => setPlatform(detectPlatform()), []);
  return platform;
}

export const PLATFORM_LABEL: Record<PlatformKey, string> = {
  macos: "macOS",
  windows: "Windows",
  linux: "Linux",
};
