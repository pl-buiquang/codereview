import { useEffect, useRef, useState } from "react";
import { Icon } from "./icons";

/** Split button shown when a review's branch has new commits. The primary action moves this
 *  review to the latest commits (drafts only); the menu starts a separate review of just what
 *  changed since this one (rebase-aware), which also works on a published review. */
export function UpdateReviewButton({
  busy,
  canUpdate,
  canReviewNew,
  onUpdate,
  onReviewNew,
}: {
  busy: boolean;
  /** Only a draft can be moved to the latest commits. */
  canUpdate: boolean;
  /** Only PR reviews can start a review of the new commits. */
  canReviewNew: boolean;
  onUpdate: () => void;
  onReviewNew: () => void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menuOpen) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [menuOpen]);

  if (!canUpdate && !canReviewNew) return null;
  return (
    <div ref={ref} className="btn-split">
      <button
        className="btn btn-sm"
        disabled={busy || !canUpdate}
        title={
          canUpdate
            ? "Move this review to the latest commits and re-anchor its comments"
            : "A published review can't move. Use the menu to review the new commits"
        }
        onClick={onUpdate}
      >
        {busy ? "Updating…" : "Update review"}
      </button>
      {canReviewNew && (
        <button
          className="btn btn-sm"
          disabled={busy}
          title="More update options"
          aria-label="More update options"
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          onClick={() => setMenuOpen((o) => !o)}
        >
          <Icon name="chev" size={11} />
        </button>
      )}
      {menuOpen && canReviewNew && (
        <div className="btn-split-menu card" role="menu">
          <button
            role="menuitem"
            title="Start a new review of only what changed since this one (rebase-aware)"
            onClick={() => {
              setMenuOpen(false);
              onReviewNew();
            }}
          >
            Review new commits as a new review
          </button>
        </div>
      )}
    </div>
  );
}
