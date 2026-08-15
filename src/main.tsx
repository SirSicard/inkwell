// Two-stage entry. Static imports are hoisted and evaluated before any module
// body runs, so a conditional shim in this file's body would load after the
// app (and after framer-motion has captured the real requestAnimationFrame).
// The dynamic import chain is what guarantees the visual-pass shim, when it
// applies at all, is installed before anything else executes. In a Tauri
// build the condition is false and this is just an async indirection.
async function boot() {
  if (import.meta.env.DEV && !('__TAURI_INTERNALS__' in window)) {
    await import('./devmock')
  }
  await import('./bootstrap')
}

void boot()
