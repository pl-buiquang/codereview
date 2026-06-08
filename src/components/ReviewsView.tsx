import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { confirmDialog } from "../lib/confirm";
import { timeAgo } from "../lib/timeAgo";
import { githubPrUrl } from "../lib/githubUrl";
import { useUIStore } from "../store";
import type { ReviewSummary } from "../lib/types";
import { OpenPrButton } from "./OpenPrButton";

type SortKey = "modified" | "created" | "comments" | "title";

const SORTS: { key: SortKey; label: string }[] = [
  { key: "modified", label: "Last modified" },
  { key: "created", label: "Recently created" },
  { key: "comments", label: "Most comments" },
  { key: "title", label: "Title (A–Z)" },
];

// Filter axes derived from each review. `key` is matched against the value the
// extractor returns; `label` is what the sidebar shows.
const STATUS_LABELS: Record<string, string> = { draft: "Draft", published: "Published" };
const ORIGIN_LABELS: Record<string, string> = { github_pr: "GitHub", local: "Local" };
const VERDICT_LABELS: Record<string, string> = {
  approve: "Approve",
  request_changes: "Request changes",
  comment: "Comment",
  none: "No verdict",
};

function statusOf(r: ReviewSummary) {
  return r.review.status;
}
function originOf(r: ReviewSummary) {
  return r.target.kind;
}
function repoOf(r: ReviewSummary) {
  return r.repo_label;
}
function verdictOf(r: ReviewSummary) {
  return r.review.event ?? "none";
}

function prUrlFor(r: ReviewSummary): string | null {
  if (r.target.kind !== "github_pr" || r.target.github_pr_number == null) return null;
  const parts = r.repo_label.split("/");
  if (parts.length !== 2 || !parts[0] || !parts[1]) return null;
  return githubPrUrl(parts[0], parts[1], r.target.github_pr_number);
}

export function ReviewsView() {
  const queryClient = useQueryClient();
  const openReview = useUIStore((s) => s.openReview);

  const reviewsQuery = useQuery({ queryKey: ["reviews", null], queryFn: () => api.listReviews(null) });

  const deleteReview = useMutation({
    mutationFn: (id: number) => api.deleteReview(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["reviews"] }),
    onError: (e) => toast.error(`Could not delete review:\n${String(e)}`),
  });

  const [sort, setSort] = useState<SortKey>("modified");
  const [statusFilter, setStatusFilter] = useState<string | null>(null);
  const [originFilter, setOriginFilter] = useState<string | null>(null);
  const [repoFilter, setRepoFilter] = useState<string | null>(null);
  const [verdictFilter, setVerdictFilter] = useState<string | null>(null);

  const all = reviewsQuery.data ?? [];

  const { statusEntries, originEntries, repoEntries, verdictEntries } = useMemo(() => {
    const tally = (get: (r: ReviewSummary) => string, labels?: Record<string, string>) => {
      const m = new Map<string, number>();
      for (const r of all) {
        const k = get(r);
        m.set(k, (m.get(k) ?? 0) + 1);
      }
      return [...m.entries()]
        .map(([key, count]) => ({ key, label: labels?.[key] ?? key, count }))
        .sort((a, b) => b.count - a.count || a.label.localeCompare(b.label));
    };
    return {
      statusEntries: tally(statusOf, STATUS_LABELS),
      originEntries: tally(originOf, ORIGIN_LABELS),
      repoEntries: tally(repoOf),
      verdictEntries: tally(verdictOf, VERDICT_LABELS),
    };
  }, [all]);

  const filtered = useMemo(() => {
    const rows = all.filter((r) => {
      if (statusFilter && statusOf(r) !== statusFilter) return false;
      if (originFilter && originOf(r) !== originFilter) return false;
      if (repoFilter && repoOf(r) !== repoFilter) return false;
      if (verdictFilter && verdictOf(r) !== verdictFilter) return false;
      return true;
    });
    const sorted = [...rows];
    sorted.sort((a, b) => {
      switch (sort) {
        case "created":
          return b.review.created_at.localeCompare(a.review.created_at);
        case "comments":
          return b.comment_count - a.comment_count || b.review.updated_at.localeCompare(a.review.updated_at);
        case "title":
          return a.target.title.localeCompare(b.target.title);
        case "modified":
        default:
          return b.review.updated_at.localeCompare(a.review.updated_at);
      }
    });
    return sorted;
  }, [all, statusFilter, originFilter, repoFilter, verdictFilter, sort]);

  return (
    <section className="main-panel inbox-panel">
      <header className="inbox-header">
        <div>
          <h2 className="inbox-h">Reviews</h2>
          <p className="muted small">
            {reviewsQuery.isLoading
              ? "Loading…"
              : `${filtered.length}${filtered.length !== all.length ? ` of ${all.length}` : ""} review${
                  all.length === 1 && filtered.length === 1 ? "" : "s"
                }`}
          </p>
        </div>
        <div className="inbox-header-actions">
          <label className="muted small sort-control">
            Sort
            <select className="sort-select" value={sort} onChange={(e) => setSort(e.target.value as SortKey)}>
              {SORTS.map((s) => (
                <option key={s.key} value={s.key}>
                  {s.label}
                </option>
              ))}
            </select>
          </label>
        </div>
      </header>

      <div className="inbox-layout">
        <aside className="inbox-filters">
          <FilterGroup title="Status" entries={statusEntries} selected={statusFilter} onSelect={setStatusFilter} />
          <FilterGroup title="Origin" entries={originEntries} selected={originFilter} onSelect={setOriginFilter} />
          <FilterGroup title="Repositories" entries={repoEntries} selected={repoFilter} onSelect={setRepoFilter} mono />
          <FilterGroup title="Verdict" entries={verdictEntries} selected={verdictFilter} onSelect={setVerdictFilter} />
        </aside>

        <div className="inbox-list">
          {reviewsQuery.isLoading && <p className="muted">Loading…</p>}
          {reviewsQuery.isError && <p className="error">Could not load reviews: {String(reviewsQuery.error)}</p>}
          {!reviewsQuery.isLoading && all.length === 0 && (
            <p className="muted">No reviews yet. Start one from a repository or the inbox.</p>
          )}
          {!reviewsQuery.isLoading && all.length > 0 && filtered.length === 0 && (
            <p className="muted">No reviews match the current filters.</p>
          )}
          {filtered.map((r) => (
            <ReviewRow
              key={r.review.id}
              summary={r}
              prUrl={prUrlFor(r)}
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
    </section>
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
  const { review, target, repo_label, comment_count } = summary;
  const kindLabel = target.kind === "github_pr" ? `PR #${target.github_pr_number}` : "local";
  return (
    <div className="review-row" onClick={onOpen}>
      <div className="review-row-main">
        <span className="review-row-title">{target.title}</span>
        <span className="muted small">
          <span className="mono">{repo_label}</span> · {kindLabel} · {comment_count} comment
          {comment_count === 1 ? "" : "s"}
          {review.event ? ` · ${review.event}` : ""} · updated {timeAgo(review.updated_at)}
        </span>
      </div>
      {prUrl && <OpenPrButton url={prUrl} size="xs" />}
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

function FilterGroup({
  title,
  entries,
  selected,
  onSelect,
  mono,
}: {
  title: string;
  entries: { key: string; label: string; count: number }[];
  selected: string | null;
  onSelect: (v: string | null) => void;
  mono?: boolean;
}) {
  if (entries.length === 0) return null;
  return (
    <div className="filter-group">
      <div className="filter-title">
        <span>{title}</span>
        {selected && (
          <button className="filter-clear" onClick={() => onSelect(null)}>
            clear
          </button>
        )}
      </div>
      <ul className="filter-items">
        {entries.map(({ key, label, count }) => (
          <li key={key}>
            <button
              className={`filter-item${key === selected ? " active" : ""}`}
              onClick={() => onSelect(key === selected ? null : key)}
            >
              <span className={mono ? "mono truncate" : "truncate"}>{label}</span>
              <span className="filter-count">{count}</span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
