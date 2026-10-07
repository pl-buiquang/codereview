import { useEffect, useState, type ReactNode } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { confirmDialog } from "../lib/confirm";
import { PR_LIST_POLL_OPTIONS, useSettingsStore, parseRepoBasePaths } from "../lib/settings";
import { timeAgo } from "../lib/timeAgo";
import { GraphView } from "./graph/GraphView";
import { OpenPrButton } from "./OpenPrButton";
import { Icon } from "./icons";
import { githubPrUrl } from "../lib/githubUrl";
import { useUIStore } from "../store";
import type { PrSummary, Repository, ReviewSummary } from "../lib/types";
import { statusLabel, statusBadgeClass } from "../lib/status";

type Tab = "graph" | "prs";

export function RepoView({ repo }: { repo: Repository }) {
  const openReview = useUIStore((s) => s.openReview);
  const queryClient = useQueryClient();
  const [tab, setTab] = useState<Tab>(repo.local_path ? "graph" : "prs");

  const reviewsQuery = useQuery({
    queryKey: ["reviews", repo.id],
    queryFn: () => api.listReviews(repo.id),
    refetchOnWindowFocus: true,
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
            : repo.local_path ?? "Repository"}
        </h1>
      </header>

      <div className="cr-tabs">
        <button
          className={`cr-tab${tab === "graph" ? " active" : ""}${!repo.local_path ? " disabled" : ""}`}
          onClick={() => repo.local_path && setTab("graph")}
          disabled={!repo.local_path}
          title={!repo.local_path ? "Graph requires a local clone" : undefined}
        >
          Graph
        </button>
        <button
          className={`cr-tab${tab === "prs" ? " active" : ""}`}
          onClick={() => setTab("prs")}
        >
          GitHub PRs
        </button>
      </div>

      {tab === "graph" ? (
        <div className="repo-body repo-body-graph">
          <GraphView repo={repo} />
        </div>
      ) : (
        <div className="repo-body">
          <PrList repo={repo} onOpen={openReview} />

          <div className="repo-reviews">
            <div className="repo-section-row">
              <h3 className="repo-section-h">Reviews</h3>
              <button
                className="btn btn-sm"
                disabled={reviewsQuery.isFetching}
                onClick={() => reviewsQuery.refetch()}
                title="Refresh reviews"
              >
                {reviewsQuery.isFetching ? (
                  <span className="spinner" />
                ) : (
                  <Icon name="refresh" size={13} />
                )}
              </button>
            </div>
            {reviewsQuery.isLoading && <p className="muted">Loading…</p>}
            {reviews.length === 0 && !reviewsQuery.isLoading && (
              <p className="muted">No reviews yet.</p>
            )}
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
        </div>
      )}
    </section>
  );
}

function PrList({ repo, onOpen }: { repo: Repository; onOpen: (id: number) => void }) {
  const queryClient = useQueryClient();
  const prListPollMs = useSettingsStore((s) => s.prListPollMs);
  const setPrListPollMs = useSettingsStore((s) => s.setPrListPollMs);

  const authQuery = useQuery({ queryKey: ["gh-auth"], queryFn: api.ghAuthStatus });
  const prsQuery = useQuery({
    queryKey: ["prs", repo.id],
    queryFn: () => api.listPrs(repo.id),
    enabled: authQuery.data === true,
    refetchInterval: prListPollMs > 0 ? prListPollMs : false,
  });

  // Re-render every 30s so the "updated Xm ago" label stays honest with polling off.
  const [, setTick] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setTick((n) => n + 1), 30_000);
    return () => clearInterval(id);
  }, []);

  const repoBasePaths = useSettingsStore((s) => s.repoBasePaths);

  const startPrReview = useMutation({
    mutationFn: (prNumber: number) => {
      if (!repo.remote_owner || !repo.remote_name) {
        throw new Error("repository has no GitHub remote");
      }
      return api.createReviewForPr(repo.remote_owner, repo.remote_name, prNumber, parseRepoBasePaths(repoBasePaths));
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
