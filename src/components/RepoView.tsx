import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { ViewType } from "react-diff-view";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { confirmDialog } from "../lib/confirm";
import { useSettingsStore } from "../lib/settings";
import { DiffViewer } from "./DiffViewer";
import { useUIStore } from "../store";
import type { PrSummary, Repository, ReviewSummary } from "../lib/types";

interface Comparison {
  base: string;
  head: string;
  threeDot: boolean;
}

type Tab = "branches" | "prs";

export function RepoView({ repo }: { repo: Repository }) {
  const repoPath = repo.path;
  const openReview = useUIStore((s) => s.openReview);
  const queryClient = useQueryClient();
  const [tab, setTab] = useState<Tab>("branches");

  const reviewsQuery = useQuery({
    queryKey: ["reviews", repo.id],
    queryFn: () => api.listReviews(repo.id),
  });

  const deleteReview = useMutation({
    mutationFn: (id: number) => api.deleteReview(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["reviews", repo.id] }),
  });

  const reviews = reviewsQuery.data ?? [];

  return (
    <section className="main-panel">
      <header className="main-header">
        <h2>
          {repo.remote_owner && repo.remote_name
            ? `${repo.remote_owner}/${repo.remote_name}`
            : repoPath}
        </h2>
      </header>

      <div className="tabs">
        <button className={tab === "branches" ? "active" : ""} onClick={() => setTab("branches")}>
          Virtual PR
        </button>
        <button className={tab === "prs" ? "active" : ""} onClick={() => setTab("prs")}>
          GitHub PRs
        </button>
      </div>

      {tab === "branches" ? (
        <BranchCompare repo={repo} />
      ) : (
        <PrList repo={repo} onOpen={openReview} />
      )}

      {reviews.length > 0 && (
        <div className="reviews-list">
          <h3>Reviews</h3>
          {reviews.map((r) => (
            <ReviewRow
              key={r.review.id}
              summary={r}
              onOpen={() => openReview(r.review.id)}
              onDelete={async () => {
                if (
                  await confirmDialog({
                    title: "Delete review",
                    message: "Delete this review and all its comments?",
                    confirmLabel: "Delete",
                    danger: true,
                  })
                )
                  deleteReview.mutate(r.review.id);
              }}
            />
          ))}
        </div>
      )}
    </section>
  );
}

function BranchCompare({ repo }: { repo: Repository }) {
  const repoPath = repo.path;
  const openReview = useUIStore((s) => s.openReview);
  const queryClient = useQueryClient();

  const branchesQuery = useQuery({
    queryKey: ["branches", repoPath],
    queryFn: () => api.listBranches(repoPath),
  });
  const branchNames = useMemo(
    () => (branchesQuery.data ?? []).map((b) => b.name),
    [branchesQuery.data],
  );

  const defaultThreeDot = useSettingsStore((s) => s.defaultThreeDot);
  const defaultViewType = useSettingsStore((s) => s.defaultViewType);
  const [base, setBase] = useState("");
  const [head, setHead] = useState("");
  const [threeDot, setThreeDot] = useState(defaultThreeDot);
  const [viewType, setViewType] = useState<ViewType>(defaultViewType);
  const [comparison, setComparison] = useState<Comparison | null>(null);

  useEffect(() => {
    if (branchNames.length === 0) return;
    const fallbackBase = repo.default_branch ?? branchNames[0];
    setBase((prev) => (prev && branchNames.includes(prev) ? prev : fallbackBase));
    setHead((prev) => {
      if (prev && branchNames.includes(prev)) return prev;
      return branchNames.find((n) => n !== fallbackBase) ?? branchNames[0];
    });
  }, [branchNames, repo.default_branch]);

  const diffQuery = useQuery({
    queryKey: ["diff", repoPath, comparison?.base, comparison?.head, comparison?.threeDot],
    queryFn: () =>
      api.diffRefs(repoPath, comparison!.base, comparison!.head, comparison!.threeDot),
    enabled: comparison != null,
  });

  const startReview = useMutation({
    mutationFn: () =>
      api.createReview({ repoId: repo.id, repoPath, baseRef: base, headRef: head, threeDot }),
    onSuccess: (review) => {
      queryClient.invalidateQueries({ queryKey: ["reviews", repo.id] });
      openReview(review.id);
    },
    onError: (e) => toast.error(String(e)),
  });

  const canCompare = base !== "" && head !== "" && base !== head;

  return (
    <>
      <div className="compare-bar">
        <span className="compare-label">New virtual PR</span>
        <label>
          base
          <select value={base} onChange={(e) => setBase(e.target.value)}>
            {branchNames.map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
        </label>
        <span className="arrow">←</span>
        <label>
          compare
          <select value={head} onChange={(e) => setHead(e.target.value)}>
            {branchNames.map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
        </label>
        <label className="checkbox" title="Diff against merge-base (GitHub PR semantics)">
          <input type="checkbox" checked={threeDot} onChange={(e) => setThreeDot(e.target.checked)} />
          merge-base
        </label>
        <button disabled={!canCompare} onClick={() => setComparison({ base, head, threeDot })}>
          Preview diff
        </button>
        <button
          className="btn-primary"
          disabled={!canCompare || startReview.isPending}
          onClick={() => startReview.mutate()}
        >
          {startReview.isPending ? "Starting…" : "Start review"}
        </button>
        <div className="view-toggle">
          <button className={viewType === "split" ? "active" : ""} onClick={() => setViewType("split")}>
            Split
          </button>
          <button
            className={viewType === "unified" ? "active" : ""}
            onClick={() => setViewType("unified")}
          >
            Unified
          </button>
        </div>
      </div>

      <div className="diff-area">
        {!comparison && (
          <p className="muted">Pick branches, then “Preview diff” or “Start review”.</p>
        )}
        {comparison && diffQuery.isLoading && <p className="muted">Loading diff…</p>}
        {comparison && diffQuery.isError && (
          <p className="error">Diff failed: {String(diffQuery.error)}</p>
        )}
        {comparison && diffQuery.data != null && (
          <DiffViewer diffText={diffQuery.data} viewType={viewType} />
        )}
      </div>
    </>
  );
}

function PrList({ repo, onOpen }: { repo: Repository; onOpen: (id: number) => void }) {
  const queryClient = useQueryClient();
  const authQuery = useQuery({ queryKey: ["gh-auth"], queryFn: api.ghAuthStatus });
  const prsQuery = useQuery({
    queryKey: ["prs", repo.path],
    queryFn: () => api.listPrs(repo.path),
    enabled: authQuery.data === true,
  });

  const startPrReview = useMutation({
    mutationFn: (prNumber: number) => api.createReviewForPr(repo.id, repo.path, prNumber),
    onSuccess: (review) => {
      queryClient.invalidateQueries({ queryKey: ["reviews", repo.id] });
      onOpen(review.id);
    },
    onError: (e) => toast.error(String(e)),
  });

  if (authQuery.isLoading) return <p className="muted">Checking GitHub auth…</p>;
  if (authQuery.data === false)
    return (
      <p className="muted">
        Not authenticated with GitHub. Run <code>gh auth login</code> in a terminal, then reopen.
      </p>
    );
  if (prsQuery.isLoading) return <p className="muted">Loading open PRs…</p>;
  if (prsQuery.isError)
    return <p className="error">Could not list PRs: {String(prsQuery.error)}</p>;

  const prs = prsQuery.data ?? [];
  if (prs.length === 0) return <p className="muted">No open pull requests.</p>;

  return (
    <div className="pr-list">
      {prs.map((pr: PrSummary) => (
        <div
          key={pr.number}
          className="pr-row"
          onClick={() => startPrReview.mutate(pr.number)}
          title="Start a review of this PR"
        >
          <div className="pr-row-main">
            <span className="pr-title">
              #{pr.number} {pr.title}
            </span>
            <span className="muted">
              {pr.author?.login ?? "unknown"} · {pr.baseRefName} ← {pr.headRefName}
            </span>
          </div>
          <button className="btn-primary" disabled={startPrReview.isPending}>
            Review
          </button>
        </div>
      ))}
    </div>
  );
}

function ReviewRow({
  summary,
  onOpen,
  onDelete,
}: {
  summary: ReviewSummary;
  onOpen: () => void;
  onDelete: () => void;
}) {
  const { review, target, comment_count } = summary;
  const kindLabel = target.kind === "github_pr" ? `PR #${target.github_pr_number}` : "local";
  return (
    <div className="review-row" onClick={onOpen}>
      <div className="review-row-main">
        <span className="review-row-title">{target.title}</span>
        <span className="muted">
          {kindLabel} · {comment_count} comment{comment_count === 1 ? "" : "s"}
          {review.event ? ` · ${review.event}` : ""}
        </span>
      </div>
      <span className={`status-badge ${review.status}`}>{review.status}</span>
      <button
        className="btn-icon"
        title="Delete review"
        onClick={(e) => {
          e.stopPropagation();
          onDelete();
        }}
      >
        ✕
      </button>
    </div>
  );
}
