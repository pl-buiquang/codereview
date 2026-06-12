import { useEffect, useMemo, useState, type ReactNode } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { ViewType } from "react-diff-view";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { confirmDialog } from "../lib/confirm";
import { PR_LIST_POLL_OPTIONS, useSettingsStore } from "../lib/settings";
import { timeAgo } from "../lib/timeAgo";
import { DiffViewer } from "./DiffViewer";
import { OpenPrButton } from "./OpenPrButton";
import { Icon } from "./icons";
import { githubPrUrl } from "../lib/githubUrl";
import { useUIStore } from "../store";
import type { PrSummary, Repository, ReviewSummary } from "../lib/types";
import { statusLabel, statusBadgeClass } from "../lib/status";

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
    <section className="cr-main">
      <header className="cr-pagehead">
        <h1 className="cr-h1 repo-title">
          {repo.remote_owner && repo.remote_name
            ? `${repo.remote_owner}/${repo.remote_name}`
            : repoPath}
        </h1>
      </header>

      <div className="cr-tabs">
        <button
          className={`cr-tab${tab === "branches" ? " active" : ""}`}
          onClick={() => setTab("branches")}
        >
          Virtual PR
        </button>
        <button
          className={`cr-tab${tab === "prs" ? " active" : ""}`}
          onClick={() => setTab("prs")}
        >
          GitHub PRs
        </button>
      </div>

      <div className="repo-body">
        {tab === "branches" ? (
          <BranchCompare repo={repo} />
        ) : (
          <PrList repo={repo} onOpen={openReview} />
        )}

        {reviews.length > 0 && (
          <div className="repo-reviews">
            <h3 className="repo-section-h">Reviews</h3>
            {reviews.map((r) => (
              <ReviewRow
                key={r.review.id}
                summary={r}
                prUrl={
                  r.target.kind === "github_pr" &&
                  repo.remote_owner &&
                  repo.remote_name &&
                  r.target.github_pr_number != null
                    ? githubPrUrl(repo.remote_owner, repo.remote_name, r.target.github_pr_number)
                    : null
                }
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
      </div>
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
      <div className="card vpr-card">
        <h3 className="repo-section-h">New virtual PR</h3>
        <div className="vpr-row">
          <label className="vpr-field">
            base
            <select className="select mono" value={base} onChange={(e) => setBase(e.target.value)}>
              {branchNames.map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </label>
          <Icon name="back" size={14} />
          <label className="vpr-field">
            compare
            <select className="select mono" value={head} onChange={(e) => setHead(e.target.value)}>
              {branchNames.map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </label>
          <label className="check" title="Diff against merge-base (GitHub PR semantics)">
            <input type="checkbox" checked={threeDot} onChange={(e) => setThreeDot(e.target.checked)} />
            merge-base
          </label>
        </div>
        <div className="vpr-row">
          <button className="btn" disabled={!canCompare} onClick={() => setComparison({ base, head, threeDot })}>
            Preview diff
          </button>
          <button
            className="btn btn-primary"
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
        <p className="vpr-helper">Pick branches, then “Preview diff” or “Start review”.</p>
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
  const prListPollMs = useSettingsStore((s) => s.prListPollMs);
  const setPrListPollMs = useSettingsStore((s) => s.setPrListPollMs);

  const authQuery = useQuery({ queryKey: ["gh-auth"], queryFn: api.ghAuthStatus });
  const prsQuery = useQuery({
    queryKey: ["prs", repo.path],
    queryFn: () => api.listPrs(repo.path),
    enabled: authQuery.data === true,
    refetchInterval: prListPollMs > 0 ? prListPollMs : false,
  });

  // Re-render every 30s so the "updated Xm ago" label stays honest with polling off.
  const [, setTick] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setTick((n) => n + 1), 30_000);
    return () => clearInterval(id);
  }, []);

  const startPrReview = useMutation({
    mutationFn: (prNumber: number) => {
      if (!repo.remote_owner || !repo.remote_name) {
        throw new Error("repository has no GitHub remote");
      }
      return api.createReviewForPr(repo.remote_owner, repo.remote_name, prNumber);
    },
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
  const prs = prsQuery.data ?? [];
  let body: ReactNode;
  if (prsQuery.isLoading) {
    body = <p className="muted">Loading open PRs…</p>;
  } else if (prsQuery.isError) {
    body = <p className="error">Could not list PRs: {String(prsQuery.error)}</p>;
  } else if (prs.length === 0) {
    body = <p className="muted">No open pull requests.</p>;
  } else {
    body = (
      <div className="pr-list">
        {prs.map((pr: PrSummary) => (
          <div
            key={pr.number}
            className="card pr-row"
            onClick={() => startPrReview.mutate(pr.number)}
            title="Start a review of this PR"
          >
            <div className="pr-row-main">
              <span className="pr-title">
                #{pr.number} {pr.title}
              </span>
              <span className="faint">
                {pr.author?.login ?? "unknown"} · {pr.baseRefName} ← {pr.headRefName}
              </span>
            </div>
            <button className="btn btn-sm btn-primary" disabled={startPrReview.isPending}>
              Review
            </button>
          </div>
        ))}
      </div>
    );
  }

  return (
    <>
      <div className="pr-list-toolbar">
        <span className="muted">Open pull requests</span>
        <span className="cr-spacer" />
        {prsQuery.dataUpdatedAt > 0 && (
          <span className="cr-sub">updated {timeAgo(prsQuery.dataUpdatedAt)}</span>
        )}
        <label className="sort-control">
          auto
          <select
            className="sort-select"
            value={prListPollMs}
            onChange={(e) => setPrListPollMs(Number(e.target.value))}
          >
            {PR_LIST_POLL_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </label>
        <button
          className="btn btn-primary"
          disabled={prsQuery.isFetching}
          onClick={() => prsQuery.refetch()}
        >
          {prsQuery.isFetching ? (
            <>
              <span className="spinner" /> Refreshing…
            </>
          ) : (
            <>
              <Icon name="refresh" size={13} /> Refresh
            </>
          )}
        </button>
      </div>
      {body}
    </>
  );
}

function ReviewRow({
  summary,
  prUrl,
  onOpen,
  onDelete,
}: {
  summary: ReviewSummary;
  prUrl: string | null;
  onOpen: () => void;
  onDelete: () => void;
}) {
  const { review, target, comment_count } = summary;
  const kindLabel = target.kind === "github_pr" ? `PR #${target.github_pr_number}` : "local";
  return (
    <div className="card rev-row" onClick={onOpen}>
      <div className="rev-main">
        <span className="rev-title">{target.title}</span>
        <div className="rev-meta">
          <span>{kindLabel}</span>
          <span className="sep">
            {comment_count} comment{comment_count === 1 ? "" : "s"}
          </span>
          {review.event && <span className="sep">{review.event}</span>}
        </div>
      </div>
      {prUrl && <OpenPrButton url={prUrl} size="xs" />}
      <span className={`badge ${statusBadgeClass(review.status)}`}>
        {statusLabel(review.status)}
      </span>
      <button
        className="btn-icon"
        title="Delete review"
        onClick={(e) => {
          e.stopPropagation();
          onDelete();
        }}
      >
        <Icon name="x" size={12} />
      </button>
    </div>
  );
}
