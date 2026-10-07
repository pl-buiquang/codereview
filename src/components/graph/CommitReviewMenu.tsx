import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "../../lib/api";
import { toast } from "../../lib/toast";
import { parseRepoBasePaths, useSettingsStore } from "../../lib/settings";
import { useUIStore } from "../../store";
import type { CommitDetail, PrSummary, RefInfo, Repository } from "../../lib/types";
import { ReviewVsDialog, useStartReview } from "./ReviewVsDialog";
import { shortSha } from "./CommitPanel";
import "./RepoSidebar.css";

export interface CommitReviewMenuProps {
  repo: Repository;
  detail: CommitDetail;
  /** Branch refs whose tip is this commit (local first). */
  tipRefs: RefInfo[];
  pullRequests: PrSummary[];
}

/** Head ref to review: a local branch at this tip, else a remote one, else the sha itself. */
export function reviewHead(detail: CommitDetail, tipRefs: RefInfo[]): string {
  return (
    tipRefs.find((r) => r.kind === "local")?.name ??
    tipRefs.find((r) => r.kind === "remote")?.name ??
    detail.sha
  );
}

export function CommitReviewMenu({ repo, detail, tipRefs, pullRequests }: CommitReviewMenuProps) {
  const queryClient = useQueryClient();
  const openReview = useUIStore((s) => s.openReview);
  const repoBasePaths = useSettingsStore((s) => s.repoBasePaths);
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);
  const [vsOpen, setVsOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  const head = reviewHead(detail, tipRefs);
  const headLabel = head === detail.sha ? shortSha(detail.sha) : head;
  const defaultBranch = repo.default_branch;
  const canVsDefault = !!defaultBranch && defaultBranch !== head;
  const parent = detail.parents[0];

  const startReview = useStartReview(repo, (id) => {
    setMenuAt(null);
    openReview(id);
  });
  const startPrReview = useMutation({
    mutationFn: (pr: PrSummary) => {
      if (!repo.remote_owner || !repo.remote_name) throw new Error("repository has no GitHub remote");
      return api.createReviewForPr(repo.remote_owner, repo.remote_name, pr.number, parseRepoBasePaths(repoBasePaths));
    },
    onSuccess: (review) => {
      queryClient.invalidateQueries({ queryKey: ["reviews", repo.id] });
      setMenuAt(null);
      openReview(review.id);
    },
    onError: (e) => toast.error(String(e)),
  });
  const busy = startReview.isPending || startPrReview.isPending;

  const reviewVsDefault = () =>
    defaultBranch && startReview.mutate({ baseRef: defaultBranch, headRef: head, threeDot: true });
  const reviewCommitOnly = () =>
    parent && startReview.mutate({ baseRef: parent, headRef: detail.sha, threeDot: false });

  const primary = pullRequests[0]
    ? { label: `Review PR #${pullRequests[0].number}`, run: () => startPrReview.mutate(pullRequests[0]) }
    : canVsDefault
      ? { label: `Review vs ${defaultBranch}`, run: reviewVsDefault }
      : parent
        ? { label: "Review this commit", run: reviewCommitOnly }
        : null;

  useEffect(() => {
    if (!menuAt) return;
    const onDown = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) setMenuAt(null);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setMenuAt(null);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [menuAt]);

  return (
    <div className="cp-review">
      <button className="btn btn-sm btn-primary" disabled={!primary || busy} onClick={() => primary?.run()}>
        {busy ? <span className="spinner" /> : (primary?.label ?? "Start review")}
      </button>
      <button
        className="btn btn-sm"
        aria-label="More review options"
        aria-haspopup="menu"
        disabled={busy}
        onClick={(e) => {
          const rect = e.currentTarget.getBoundingClientRect();
          setMenuAt(menuAt ? null : { x: Math.max(8, rect.right - 260), y: rect.bottom + 2 });
        }}
      >
        ▾
      </button>
      {menuAt &&
        createPortal(
          <div
            className="rs-menu"
            ref={menuRef}
            role="menu"
            aria-label="Review options"
            style={{ top: menuAt.y, left: menuAt.x, minWidth: 260 }}
          >
            {pullRequests.map((pr) => (
              <button key={pr.number} className="rs-menu-item" role="menuitem" onClick={() => startPrReview.mutate(pr)}>
                GitHub PR #{pr.number}
              </button>
            ))}
            <div className="rs-menu-title mono">{headLabel}</div>
            <button className="rs-menu-item" role="menuitem" disabled={!canVsDefault} onClick={reviewVsDefault}>
              vs <span className="mono">{defaultBranch ?? "default branch"}</span>
            </button>
            <button
              className="rs-menu-item"
              role="menuitem"
              onClick={() => {
                setMenuAt(null);
                setVsOpen(true);
              }}
            >
              vs…
            </button>
            <button className="rs-menu-item" role="menuitem" disabled={!parent} onClick={reviewCommitOnly}>
              This commit only (vs parent <span className="mono">{parent ? shortSha(parent) : "—"}</span>)
            </button>
          </div>,
          document.body,
        )}
      {vsOpen && (
        <ReviewVsDialog
          repo={repo}
          head={head}
          onClose={() => setVsOpen(false)}
          onReviewCreated={(id) => {
            setVsOpen(false);
            openReview(id);
          }}
        />
      )}
    </div>
  );
}
