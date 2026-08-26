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

// Modifier-only hotkeys are stored as bare tokens, not "+"-joined combos, and
// have fixed display names. Mac-only feature, so the glyphs are safe.
const MOD_TOKEN_NAMES: Record<string, string> = {
  fn: "Fn \u{1F310}",
  right_cmd: "Right \u2318",
  right_opt: "Right \u2325",
  right_ctrl: "Right \u2303",
}

export function formatHotkey(hotkey: string): string {
  if (!hotkey) return ""
  if (hotkey in MOD_TOKEN_NAMES) return MOD_TOKEN_NAMES[hotkey]
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
