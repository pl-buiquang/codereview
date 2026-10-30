import { useEffect } from "react";
import { createPortal } from "react-dom";
import { toast } from "../../lib/toast";
import type { RefInfo } from "../../lib/types";
import "./RepoSidebar.css";

export function copyToClipboard(text: string) {
  navigator.clipboard.writeText(text).then(
    () => toast.info(`Copied ${text}`),
    (e) => toast.error(`Could not copy: ${String(e)}`),
  );
}

const KIND_LABEL: Record<RefInfo["kind"], string> = { local: "branch", remote: "remote", tag: "tag" };

/** Picks one of several branch/tag names pointing at a commit and copies it. */
export function CopyRefDialog({ refs, onClose }: { refs: RefInfo[]; onClose: () => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  return createPortal(
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal rvd-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Copy name"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-header">
          <h3>Copy name</h3>
        </div>
        <div className="crd-list" role="listbox" aria-label="Branches and tags">
          {refs.map((r) => (
            <button
              key={`${r.kind}:${r.name}`}
              className="rs-menu-item crd-item"
              role="option"
              aria-selected={false}
              onClick={() => {
                copyToClipboard(r.name);
                onClose();
              }}
            >
              <span className="mono">{r.name}</span>
              <span className="muted crd-kind">{KIND_LABEL[r.kind]}</span>
            </button>
          ))}
        </div>
        <div className="modal-actions">
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
