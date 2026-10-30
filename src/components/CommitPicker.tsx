import { useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { commitRange, isSelected, selectCommit, type CommitSelection } from "../lib/commitRange";
import { timeAgo } from "../lib/timeAgo";
import { toast } from "../lib/toast";
import { useUIStore } from "../store";
import { CommitRangeQuickView } from "./CommitRangeQuickView";
import { shortSha } from "./graph/CommitPanel";
import { Icon } from "./icons";

/**
 * Header button listing a PR review's commits (pinned base..head). Click one commit, then
 * shift-click another to select a range; the range can be previewed in a quick view or
 * turned into a local review (parent of the oldest commit..the newest, two-dot).
 */
export function CommitPicker({
  reviewId,
  repoId,
  headSha,
  hasClone,
}: {
  reviewId: number;
  repoId: number;
  /** The review's pinned head, so the list refetches after the pin moves. */
  headSha: string | null;
  hasClone: boolean;
}) {
  const queryClient = useQueryClient();
  const openReview = useUIStore((s) => s.openReview);
  const [open, setOpen] = useState(false);
  const [sel, setSel] = useState<CommitSelection | null>(null);
  const [quickView, setQuickView] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  // Lazy: opening a review shouldn't run git (or fetch missing PR commits) unprompted.
  const query = useQuery({
    queryKey: ["review-commits", reviewId, headSha],
    queryFn: () => api.reviewCommits(reviewId),
    enabled: hasClone && (open || quickView),
    staleTime: 60_000,
    retry: false,
  });
  const commits = query.data ?? [];

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  const range = sel && sel.anchor < commits.length && sel.focus < commits.length ? commitRange(commits, sel) : null;

  const create = useMutation({
    mutationFn: (r: { base: string; head: string }) =>
      api.createReview({ repoId, baseRef: r.base, headRef: r.head, threeDot: false }),
    onSuccess: (review) => {
      queryClient.invalidateQueries({ queryKey: ["reviews", repoId] });
      setQuickView(false);
      setOpen(false);
      openReview(review.id);
    },
    onError: (e) => toast.error(String(e)),
  });
  const createRange = () => range?.base && create.mutate({ base: range.base, head: range.head });

  return (
    <div ref={ref} className="commit-picker">
      <button
        className="btn btn-sm"
        disabled={!hasClone}
        aria-haspopup="dialog"
        aria-expanded={open}
        title={hasClone ? "List this PR's commits and review a range of them" : "Needs a local clone of the repository"}
        onClick={() => setOpen((o) => !o)}
      >
        Commits{query.data ? ` (${query.data.length})` : ""} <Icon name="chev" size={11} />
      </button>
      {open && (
        <div className="commit-picker-menu card" role="dialog" aria-label="Commits">
          {query.isLoading && <p className="muted commit-picker-note">Loading commits…</p>}
          {query.isError && <p className="muted commit-picker-note">Could not load commits ({String(query.error)}).</p>}
          {query.data?.length === 0 && <p className="muted commit-picker-note">No commits.</p>}
          <ul className="commit-picker-list">
            {commits.map((c, i) => (
              <li key={c.sha}>
                <button
                  type="button"
                  className={`commit-picker-row${isSelected(sel, i) ? " selected" : ""}`}
                  aria-pressed={isSelected(sel, i)}
                  title={`${c.subject}\n${c.author_name}`}
                  onClick={(e) => setSel((prev) => selectCommit(prev, i, e.shiftKey))}
                >
                  <span className="mono commit-picker-sha">{shortSha(c.sha)}</span>
                  <span className="commit-picker-subject">{c.subject}</span>
                  <span className="muted commit-picker-meta">
                    {c.author_name} · {timeAgo(c.author_time * 1000)}
                  </span>
                </button>
              </li>
            ))}
          </ul>
          <div className="commit-picker-footer">
            <span className="muted">
              {range
                ? `${range.count} selected`
                : commits.length > 1
                  ? "Click a commit, shift-click to select a range"
                  : "Click a commit to select it"}
            </span>
            <button
              className="btn btn-sm"
              disabled={!range?.base}
              onClick={() => {
                setOpen(false);
                setQuickView(true);
              }}
            >
              Quick view
            </button>
            <button
              className="btn btn-sm btn-primary"
              disabled={!range?.base || create.isPending}
              title={range && !range.base ? "The oldest selected commit has no parent" : "Create a local review on this range"}
              onClick={createRange}
            >
              {create.isPending ? "Creating…" : "Create review"}
            </button>
          </div>
        </div>
      )}
      {quickView && range?.base && (
        <CommitRangeQuickView
          repoId={repoId}
          base={range.base}
          head={range.head}
          count={range.count}
          busy={create.isPending}
          onClose={() => setQuickView(false)}
          onCreate={createRange}
        />
      )}
    </div>
  );
}
