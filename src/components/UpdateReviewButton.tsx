import { useEffect, useRef, useState } from "react";
import { Icon } from "./icons";

/** Split button shown when a review's branch has new commits. The primary action moves this
 *  review to the latest commits (drafts only); the menu opens a quick view of just what changed
 *  since this one (rebase-aware), from which a separate review can be created; it also works on a
 *  published review. */
export function UpdateReviewButton({
  busy,
  canUpdate,
  canQuickView,
  onUpdate,
  onQuickView,
}: {
  busy: boolean;
  /** Only a draft can be moved to the latest commits. */
  canUpdate: boolean;
  /** Only PR reviews can preview the new commits. */
  canQuickView: boolean;
  onUpdate: () => void;
  onQuickView: () => void;
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

  if (!canUpdate && !canQuickView) return null;
  return (
    <div ref={ref} className="btn-split">
      <button
        className="btn btn-sm"
        disabled={busy || !canUpdate}
        title={
          canUpdate
            ? "Move this review to the latest commits and re-anchor its comments"
            : "A published review can't move. Use the menu to view the new commits"
        }
        onClick={onUpdate}
      >
        {busy ? "Updating…" : "Update review"}
      </button>
      {canQuickView && (
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
      {menuOpen && canQuickView && (
        <div className="btn-split-menu card" role="menu">
          <button
            role="menuitem"
            title="Preview only what changed since this review (rebase-aware), then create a review from it"
            onClick={() => {
              setMenuOpen(false);
              onQuickView();
            }}
          >
            Quick view new commits
          </button>
        </div>
      )}
    </div>
  );
}
