import { useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "../state/toasts"

/** Mirrors successor::Audience in src-tauri/src/successor.rs. */
type Audience = "apple_silicon" | "windows" | "intel_mac" | "linux"

/// Shown until it is dismissed or its link is used, then never again. Kept in
/// the webview's storage rather than settings.json: it is not a setting, and
/// Inkwell 1.0 imports settings.json.
const DISMISSED_KEY = "inkwell.successor-notice.dismissed"

const COPY: Record<Audience, { title: string; body: string; link: string }> = {
  apple_silicon: {
    title: "Inkwell 1.0 is out",
    body: "It is a new app, so this version cannot update itself to it. Download it from the Inkwell website. It needs macOS 26 or later.",
    link: "Get Inkwell 1.0",
  },
  windows: {
    title: "Inkwell 1.0 is out",
    body: "It is a new app, so this version cannot update itself to it. Download it from the Inkwell website.",
    link: "Get Inkwell 1.0",
  },
  intel_mac: {
    title: "This is the last version for this Mac",
    body: "Inkwell 1.0 needs a Mac with Apple silicon. This version keeps working as it is, but it will get no further updates.",
    link: "Inkwell website",
  },
  linux: {
    title: "This is the last version for Linux",
    body: "Inkwell 1.0 is for Mac and Windows. This version keeps working as it is, but it will get no further updates.",
    link: "Inkwell website",
  },
}

function wasDismissed(): boolean {
  try {
    return localStorage.getItem(DISMISSED_KEY) === "1"
  } catch {
    // Unreadable storage shows the notice again, which is the safe direction.
    return false
  }
}

/** The last 0.2 release's one notice: where Inkwell goes next. */
export function SuccessorNotice() {
  const [audience, setAudience] = useState<Audience | null>(null)

  useEffect(() => {
    if (wasDismissed()) return
    invoke<Audience>("successor_audience")
      .then(setAudience)
      .catch((e) => toast(`Could not check for Inkwell 1.0: ${e}`, "warning"))
  }, [])

  if (!audience) return null
  const copy = COPY[audience]

  const dismiss = () => {
    setAudience(null)
    try {
      localStorage.setItem(DISMISSED_KEY, "1")
    } catch (e) {
      toast(`Could not save that this notice was closed, so it will show again: ${e}`, "warning")
    }
  }

  const openSite = () => {
    invoke("open_successor_site")
      .then(dismiss)
      .catch((e) => toast(String(e), "warning"))
  }

  return (
    <div
      role="status"
      className="flex items-start gap-3 px-5 py-3 border-b border-border bg-bg-surface"
    >
      <div className="flex-1 space-y-1">
        <p className="text-sm font-medium text-text-primary">{copy.title}</p>
        <p className="text-xs text-text-secondary leading-relaxed">{copy.body}</p>
      </div>
      <div className="flex items-center gap-2 shrink-0">
        <button
          onClick={openSite}
          className="px-3 py-1.5 text-xs font-medium text-white bg-accent hover:bg-accent/90 rounded-lg transition-colors"
        >
          {copy.link}
        </button>
        <button
          onClick={dismiss}
          className="px-3 py-1.5 text-xs text-text-tertiary hover:text-text-secondary transition-colors"
        >
          Dismiss
        </button>
      </div>
    </div>
  )
}
