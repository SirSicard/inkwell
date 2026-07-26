"use client";

import { GITHUB_RELEASES_LATEST_URL } from "@/lib/constants";
import { PLATFORM_LABEL, usePlatform } from "@/lib/platform";
import { AppleIcon, DownloadIcon, LinuxIcon, WindowsIcon } from "./icons";

/**
 * Primary call to action. Always points at the GitHub "latest release" page
 * rather than a guessed asset filename. Release artifacts are produced by CI
 * per target and the exact names change between builds, so linking the page is
 * the only link that cannot rot.
 */
export default function DownloadButton({
  className = "",
}: {
  className?: string;
}) {
  const platform = usePlatform();

  const label = platform
    ? `Download for ${PLATFORM_LABEL[platform]}`
    : "Download Inkwell";

  const Icon =
    platform === "macos"
      ? AppleIcon
      : platform === "windows"
        ? WindowsIcon
        : platform === "linux"
          ? LinuxIcon
          : DownloadIcon;

  return (
    <a
      href={GITHUB_RELEASES_LATEST_URL}
      className={`btn btn-primary ${className}`}
    >
      <Icon size={17} />
      {label}
    </a>
  );
}
