import { create } from "zustand";

export interface ConfirmOptions {
  title: string;
  message?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  /** Style the confirm button as destructive. */
  danger?: boolean;
}

interface ConfirmRequest extends ConfirmOptions {
  resolve: (ok: boolean) => void;
}

interface ConfirmState {
  request: ConfirmRequest | null;
  open: (req: ConfirmRequest) => void;
  /** Resolve the pending request and close the dialog. */
  settle: (ok: boolean) => void;
}

export const useConfirmStore = create<ConfirmState>((set, get) => ({
  request: null,
  open: (req) =>
    set((s) => {
      // If a dialog is somehow already open, treat it as cancelled so its
      // promise never hangs.
      if (s.request) s.request.resolve(false);
      return { request: req };
    }),
  settle: (ok) => {
    const { request } = get();
    if (request) request.resolve(ok);
    set({ request: null });
  },
}));

/**
 * In-app replacement for `window.confirm`. Returns a promise that resolves to
 * `true` if the user confirms, `false` otherwise.
 */
export function confirmDialog(opts: ConfirmOptions): Promise<boolean> {
  return new Promise((resolve) => {
    useConfirmStore.getState().open({ ...opts, resolve });
  });
}
