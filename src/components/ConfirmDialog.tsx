import { useConfirmStore } from "../lib/confirm";

/** Single app-wide confirm modal driven by the confirm store. */
export function ConfirmDialog() {
  const request = useConfirmStore((s) => s.request);
  const settle = useConfirmStore((s) => s.settle);
  if (!request) return null;

  const {
    title,
    message,
    confirmLabel = "Confirm",
    cancelLabel = "Cancel",
    danger,
  } = request;

  return (
    <div className="modal-backdrop" onClick={() => settle(false)}>
      <div
        className="modal confirm-modal"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
      >
        <div className="modal-header">
          <h3>{title}</h3>
        </div>
        {message && <p className="confirm-message">{message}</p>}
        <div className="modal-actions">
          <button onClick={() => settle(false)}>{cancelLabel}</button>
          <button
            className={danger ? "btn-danger" : "btn-primary"}
            autoFocus
            onClick={() => settle(true)}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
