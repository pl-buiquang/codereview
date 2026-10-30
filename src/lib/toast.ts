import { create } from "zustand";

export type ToastKind = "error" | "warning" | "success" | "info";

export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
}

interface ToastState {
  toasts: Toast[];
  /** Add a toast and return its id. */
  push: (kind: ToastKind, message: string) => number;
  dismiss: (id: number) => void;
}

let nextId = 1;

export const useToastStore = create<ToastState>((set) => ({
  toasts: [],
  push: (kind, message) => {
    const id = nextId++;
    set((s) => ({ toasts: [...s.toasts, { id, kind, message }] }));
    return id;
  },
  dismiss: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
}));

function show(kind: ToastKind, message: string, ttlMs: number): number {
  const id = useToastStore.getState().push(kind, message);
  if (ttlMs > 0) {
    setTimeout(() => useToastStore.getState().dismiss(id), ttlMs);
  }
  return id;
}

/** App-wide notifications. Usable outside React (e.g. mutation `onError`). */
export const toast = {
  error: (message: string) => show("error", message, 8000),
  /** The action worked, but only partly or with a caveat worth reading. */
  warning: (message: string) => show("warning", message, 6000),
  /** The action changed something on GitHub or a remote. */
  success: (message: string) => show("success", message, 4000),
  /** Neutral notice: nothing failed, nothing was changed or needs attention. */
  info: (message: string) => show("info", message, 4000),
};
