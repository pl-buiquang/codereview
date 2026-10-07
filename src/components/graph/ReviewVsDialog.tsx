import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../../lib/api";
import { toast } from "../../lib/toast";
import { useSettingsStore } from "../../lib/settings";
import { useUIStore } from "../../store";
import type { Repository } from "../../lib/types";
import "./RepoSidebar.css";

/**
 * Creates a local review between two refs, then invalidates the repo's review
 * list and opens the new review (falls back to the UI store's `openReview`).
 */
export function useStartReview(repo: Repository, onReviewCreated?: (reviewId: number) => void) {
  const queryClient = useQueryClient();
  const storeOpenReview = useUIStore((s) => s.openReview);
  return useMutation({
    mutationFn: (args: { baseRef: string; headRef: string; threeDot: boolean }) =>
      api.createReview({ repoId: repo.id, ...args }),
    onSuccess: (review) => {
      queryClient.invalidateQueries({ queryKey: ["reviews", repo.id] });
      (onReviewCreated ?? storeOpenReview)(review.id);
    },
    onError: (e) => toast.error(String(e)),
  });
}

export interface ReviewVsDialogProps {
  repo: Repository;
  head: string;
  onClose: () => void;
  onReviewCreated?: (reviewId: number) => void;
}

export function ReviewVsDialog({ repo, head, onClose, onReviewCreated }: ReviewVsDialogProps) {
  const defaultThreeDot = useSettingsStore((s) => s.defaultThreeDot);
  const [threeDot, setThreeDot] = useState(defaultThreeDot);
  const [base, setBase] = useState("");
  const startReview = useStartReview(repo, onReviewCreated);

  const branchesQuery = useQuery({
    queryKey: ["branches", repo.id],
    queryFn: () => api.listBranches(repo.id),
    enabled: repo.local_path != null,
  });
  const baseOptions = useMemo(
    () => (branchesQuery.data ?? []).map((b) => b.name).filter((n) => n !== head),
    [branchesQuery.data, head],
  );

  useEffect(() => {
    if (baseOptions.length === 0) return;
    setBase((prev) => {
      if (prev && baseOptions.includes(prev)) return prev;
      if (repo.default_branch && baseOptions.includes(repo.default_branch)) return repo.default_branch;
      return baseOptions[0];
    });
  }, [baseOptions, repo.default_branch]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  const canConfirm = base !== "" && base !== head && !startReview.isPending;

  return createPortal(
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal rvd-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Review vs"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-header">
          <h3>
            Review <span className="mono">{head}</span> vs…
          </h3>
        </div>
        <div className="rvd-body">
          <label className="rvd-field">
            base
            <select
              className="select mono"
              aria-label="Base branch"
              value={base}
              onChange={(e) => setBase(e.target.value)}
            >
              {baseOptions.map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </label>
          <label className="check" title="Diff against merge-base (GitHub PR semantics)">
            <input
              type="checkbox"
              checked={threeDot}
              onChange={(e) => setThreeDot(e.target.checked)}
            />
            merge-base
          </label>
          {branchesQuery.isLoading && <p className="muted">Loading branches…</p>}
          {branchesQuery.isError && (
            <p className="error">Could not list branches: {String(branchesQuery.error)}</p>
          )}
        </div>
        <div className="modal-actions">
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            className="btn btn-primary"
            disabled={!canConfirm}
            onClick={() => startReview.mutate({ baseRef: base, headRef: head, threeDot })}
          >
            {startReview.isPending ? "Starting…" : "Start review"}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
