import { create } from "zustand"
import type { Toast } from "../types"

/// Toasts used to be local state inside App.tsx, which meant anything outside
/// the component tree — the settings store, an event listener — had no way to
/// tell the user something failed. That is why roughly forty invoke() calls
/// ended in `.catch(() => {})`: there was nowhere for the error to go.

let nextId = 1

interface ToastState {
  toasts: Toast[]
  push: (message: string, type?: Toast["type"]) => void
  dismiss: (id: number) => void
}

export const useToasts = create<ToastState>((set) => ({
  toasts: [],
  push: (message, type = "error") => {
    const id = nextId++
    set((s) => ({ toasts: [...s.toasts, { id, message, type }] }))
    // Errors stay long enough to read; confirmations get out of the way.
    const ttl = type === "error" ? 6000 : 3500
    setTimeout(() => {
      set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }))
    }, ttl)
  },
  dismiss: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
}))

/// For use outside React (event handlers, stores).
export const toast = (message: string, type?: Toast["type"]) =>
  useToasts.getState().push(message, type)
