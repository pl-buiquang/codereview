import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useUIStore } from "../../store";
import type { Repository } from "../../lib/types";
import { ReviewVsDialog, useStartReview } from "./ReviewVsDialog";
import "./RepoSidebar.css";

export { useStartReview };

export interface MenuAnchor {
  x: number;
  y: number;
}

export interface BranchActionsMenuProps {
  repo: Repository;
  branch: string;
  anchor: MenuAnchor;
  onClose: () => void;
  onReviewCreated?: (reviewId: number) => void;
}

export function BranchActionsMenu({
  repo,
  branch,
  anchor,
  onClose,
  onReviewCreated,
}: BranchActionsMenuProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const storeOpenReview = useUIStore((s) => s.openReview);
  const openCreated = (id: number) => {
    (onReviewCreated ?? storeOpenReview)(id);
    onClose();
  };
  const startReview = useStartReview(repo, openCreated);
  const defaultBranch = repo.default_branch;
  const canReviewVsDefault = defaultBranch != null && defaultBranch !== branch;

  useEffect(() => {
    if (dialogOpen) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose, dialogOpen]);

  if (dialogOpen) {
    return (
      <ReviewVsDialog
        repo={repo}
        head={branch}
        onClose={onClose}
        onReviewCreated={openCreated}
      />
    );
  }

  return createPortal(
    <div
      className="rs-menu"
      ref={ref}
      role="menu"
      aria-label={`Actions for ${branch}`}
      style={{ top: anchor.y, left: anchor.x }}
      onClick={(e) => e.stopPropagation()}
    >
      <div className="rs-menu-title mono">{branch}</div>
      <button
        className="rs-menu-item"
        role="menuitem"
        disabled={!canReviewVsDefault || startReview.isPending}
        title={
          defaultBranch == null
            ? "Repository has no default branch"
            : defaultBranch === branch
              ? "This is the default branch"
              : undefined
        }
        onClick={() => {
          if (!defaultBranch) return;
          startReview.mutate({ baseRef: defaultBranch, headRef: branch, threeDot: true });
        }}
      >
        Review vs <span className="mono">{defaultBranch ?? "default branch"}</span>
      </button>
      <button className="rs-menu-item" role="menuitem" onClick={() => setDialogOpen(true)}>
        Review vs…
      </button>
    </div>,
    document.body,
  );
}
