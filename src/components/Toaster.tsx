import { useToastStore } from "../lib/toast";

/** Renders the stack of active toasts. Click (or ✕) to dismiss early. */
export function Toaster() {
  const toasts = useToastStore((s) => s.toasts);
  const dismiss = useToastStore((s) => s.dismiss);
  if (toasts.length === 0) return null;
  return (
    <div className="toast-stack">
      {toasts.map((t) => (
        <div key={t.id} className={`toast ${t.kind}`} role="status">
          <span className="toast-message">{t.message}</span>
          <button
            className="toast-close"
            aria-label="Dismiss notification"
            onClick={() => dismiss(t.id)}
          >
            ✕
          </button>
        </div>
      ))}
    </div>
  );
}
