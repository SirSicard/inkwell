// Hotkeys are stored as lowercase "+"-joined tokens (e.g. "super+shift+space").
// The Cmd key serializes as "super", which must never be shown to a Mac user as
// the literal word, because macOS expects the glyphs, in modifier order.

const IS_MAC = typeof navigator !== "undefined" && navigator.userAgent.includes("Mac")

const MAC_SYMBOLS: Record<string, string> = {
  ctrl: "⌃",
  alt: "⌥",
  shift: "⇧",
  super: "⌘",
}

const PC_NAMES: Record<string, string> = {
  ctrl: "Ctrl",
  alt: "Alt",
  shift: "Shift",
  super: "Win",
}

// macOS renders modifiers in a fixed order regardless of how they were captured.
const MAC_ORDER = ["ctrl", "alt", "shift", "super"]

export function formatHotkey(hotkey: string): string {
  if (!hotkey) return ""
  const tokens = hotkey.split("+").map((t) => t.trim().toLowerCase()).filter(Boolean)
  const mods = tokens.filter((t) => t in MAC_SYMBOLS)
  const keys = tokens.filter((t) => !(t in MAC_SYMBOLS))
  const key = keys.map((k) => k.charAt(0).toUpperCase() + k.slice(1)).join(" ")

  if (IS_MAC) {
    const ordered = MAC_ORDER.filter((m) => mods.includes(m))
    return `${ordered.map((m) => MAC_SYMBOLS[m]).join("")}${key}`
  }
  return [...mods.map((m) => PC_NAMES[m]), key].filter(Boolean).join(" + ")
}
