/**
 * Theme resolution.
 *
 * Light mode originally shipped as a bare `prefers-color-scheme` media query,
 * which meant the app followed the OS and offered no way to disagree with it:
 * on a light desktop there was no route back to the dark UI. That is a
 * constraint dressed up as a preference.
 *
 * "system" stays the default, but it is resolved here to a concrete value and
 * stamped onto the root element, so the CSS only ever has to match one explicit
 * attribute. Choosing light or dark simply stops consulting the OS.
 */

export type ThemeChoice = "system" | "light" | "dark"

const QUERY = "(prefers-color-scheme: light)"

function systemIsLight(): boolean {
  return typeof window !== "undefined" && window.matchMedia(QUERY).matches
}

function resolve(choice: ThemeChoice): "light" | "dark" {
  if (choice === "system") return systemIsLight() ? "light" : "dark"
  return choice
}

export function applyTheme(choice: ThemeChoice) {
  document.documentElement.setAttribute("data-theme", resolve(choice))
}

/**
 * Apply now and keep following the OS while the choice is "system".
 * Returns an unsubscribe so callers can swap choices without stacking listeners.
 */
export function watchTheme(choice: ThemeChoice): () => void {
  applyTheme(choice)

  if (choice !== "system" || typeof window === "undefined") return () => {}

  const mq = window.matchMedia(QUERY)
  const onChange = () => applyTheme("system")
  mq.addEventListener("change", onChange)
  return () => mq.removeEventListener("change", onChange)
}
