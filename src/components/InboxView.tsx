import { useMemo } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { parseBotLogins, parseRepoBasePaths, useSettingsStore } from "../lib/settings";
import { useSyncNow, useSyncScope } from "../lib/sync";
import { timeAgo } from "../lib/timeAgo";
import { useUIStore } from "../store";
import type { InboxItem } from "../lib/types";
import { InboxItemRow, type RowVariant } from "./InboxItemRow";
import { RepoName } from "./RepoName";
import { Icon, type IconName } from "./icons";
import { SyncButton } from "./SyncButton";

type BucketKey = "needs-you" | "authored" | "team-review" | "bots" | "visited" | "closed";

const TABS: { key: BucketKey; label: string; icon: IconName; variant: RowVariant }[] = [
  { key: "needs-you", label: "Needs you", icon: "flame", variant: "inbox" },
  { key: "authored", label: "Authored", icon: "inbox", variant: "inbox" },
  { key: "team-review", label: "Team review", icon: "team", variant: "inbox" },
  { key: "bots", label: "Bots", icon: "bot", variant: "inbox" },
  { key: "visited", label: "Visited", icon: "eye", variant: "visited" },
  { key: "closed", label: "Closed", icon: "closed", variant: "closed" },
];

// GitHub's PullRequestReviewDecision enum (null for issues / draft PRs / repos
// without required reviews). Ordered most-actionable-first for the rail.
const REVIEW_ORDER: Record<string, number> = {
  REVIEW_REQUIRED: 0,
  CHANGES_REQUESTED: 1,
  APPROVED: 2,
};

function reviewLabel(decision: string): string {
  return decision.toLowerCase().replace(/_/g, " ");
}

// Draft is tracked separately from state on GitHub, but the rail treats it as
// one "Status" facet since a draft PR is never also "open" to a reviewer.
function statusKey(item: InboxItem): string {
  if (item.is_draft) return "draft";
  return item.state ?? "open";
}

function isVisited(item: InboxItem): boolean {
  return !!item.engaged_at && item.engaged_at >= item.updated_at;
}

/** Mine == authored by the viewer; my own PRs are tracked on the board by
 *  default, so they never show a manual Track button. */
function isMine(item: InboxItem): boolean {
  return item.reasons.some((r) => r.reason === "author");
}

function bucketFor(item: InboxItem, bots: Set<string>): BucketKey {
  if (isVisited(item)) return "visited";
  if (item.author_login && bots.has(item.author_login.toLowerCase())) return "bots";
  const reasons = item.reasons.map((r) => r.reason);
  if (reasons.includes("mention") || reasons.includes("direct_review") || reasons.includes("assigned")) {
    return "needs-you";
  }
  // Your own PRs belong in Authored even when a team you're on is also requested
  // for review — otherwise the team_review reason would pull them into Team review.
  if (reasons.includes("author")) return "authored";
  if (reasons.includes("team_review")) return "team-review";
  return "authored";
}

export function InboxView() {
  const queryClient = useQueryClient();
  const openReview = useUIStore((s) => s.openReview);
  const botLogins = useSettingsStore((s) => s.botLogins);
  const repoBasePaths = useSettingsStore((s) => s.repoBasePaths);
  const onHome = useUIStore((s) => s.activeTabId === "home" && s.homeRepoId == null);
  const filters = useUIStore((s) => s.inboxFilters);
  const setInboxFilters = useUIStore((s) => s.setInboxFilters);
  const {
    repo: repoFilter,
    author: authorFilter,
    type: typeFilter,
    review: reviewFilter,
    status: statusFilter,
    tracking: trackingFilter,
  } = filters;
  const active: BucketKey = TABS.find((t) => t.key === filters.bucket)?.key ?? "needs-you";
  const setActive = (bucket: BucketKey) => setInboxFilters({ bucket });
  const setRepoFilter = (repo: string | null) => setInboxFilters({ repo });
  const setAuthorFilter = (author: string | null) => setInboxFilters({ author });
  const setTypeFilter = (type: string | null) => setInboxFilters({ type });
  const setReviewFilter = (review: string | null) => setInboxFilters({ review });
  const setStatusFilter = (status: string | null) => setInboxFilters({ status });
  const setTrackingFilter = (tracking: string | null) => setInboxFilters({ tracking });

  const inboxQuery = useQuery({ queryKey: ["inbox"], queryFn: api.listInbox });
  const closedQuery = useQuery({ queryKey: ["closed"], queryFn: api.listClosed });
  const metaQuery = useQuery({ queryKey: ["inbox-meta"], queryFn: api.inboxMeta });
  const reviewsQuery = useQuery({ queryKey: ["reviews", null], queryFn: () => api.listReviews(null) });
  const trackedQuery = useQuery({ queryKey: ["board-tracked"], queryFn: api.boardTrackedIds });
  const trackedIds = useMemo(() => new Set(trackedQuery.data ?? []), [trackedQuery.data]);

  // Latest existing review per GitHub PR, keyed "owner/name#number" (repo_label
  // is owner/name for remote targets, matching InboxItem.repo). list_reviews is
  // newest-first, so the first id seen per key is the most recent review.
  const reviewByPr = useMemo(() => {
    const m = new Map<string, number>();
    for (const s of reviewsQuery.data ?? []) {
      if (s.target.kind !== "github_pr" || s.target.github_pr_number == null) continue;
      const key = `${s.repo_label}#${s.target.github_pr_number}`;
      if (!m.has(key)) m.set(key, s.review.id);
    }
    return m;
  }, [reviewsQuery.data]);

  const invalidate = () => {
    queryClient.invalidateQueries({ queryKey: ["inbox"] });
    queryClient.invalidateQueries({ queryKey: ["closed"] });
    queryClient.invalidateQueries({ queryKey: ["archive"] });
    queryClient.invalidateQueries({ queryKey: ["inbox-meta"] });
  };

  const sync = useSyncNow("inbox");

  const onErr = (e: unknown) => toast.error(String(e));
  const engage = useMutation({ mutationFn: api.engageItem, onSuccess: invalidate, onError: onErr });
  const unengage = useMutation({ mutationFn: api.unengageItem, onSuccess: invalidate, onError: onErr });
  const untrack = useMutation({ mutationFn: api.untrackItem, onSuccess: invalidate, onError: onErr });
  const track = useMutation({
    mutationFn: (item: InboxItem) => api.boardTrack(item.url),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["board-tracked"] });
      queryClient.invalidateQueries({ queryKey: ["board"] });
      toast.success("Tracked on the board");
    },
    onError: (e) => toast.error(`Could not track:\n${String(e)}`),
  });

  const openPr = useMutation({
    mutationFn: (item: InboxItem) => {
      const [owner, name] = item.repo.split("/");
      return api.openPrReview(item.id, owner, name, item.number, parseRepoBasePaths(repoBasePaths));
    },
    onSuccess: (review) => {
      invalidate();
      queryClient.invalidateQueries({ queryKey: ["reviews"] });
      openReview(review.id);
    },
    onError: (e) => toast.error(`Could not open review:\n${String(e)}`),
  });

  const bots = useMemo(() => parseBotLogins(botLogins), [botLogins]);
  const inboxItems = inboxQuery.data ?? [];
  const closedItems = closedQuery.data ?? [];

  const buckets = useMemo(() => {
    const b: Record<BucketKey, InboxItem[]> = {
      "needs-you": [],
      authored: [],
      "team-review": [],
      bots: [],
      visited: [],
      closed: closedItems,
    };
    for (const item of inboxItems) b[bucketFor(item, bots)].push(item);
    return b;
  }, [inboxItems, closedItems, bots]);

  const tabItems = buckets[active];

  // Build sidebar filter counts from the active bucket.
  const { repoEntries, authorEntries, typeEntries, reviewEntries, statusEntries, trackingEntries } = useMemo(() => {
    const repo = new Map<string, number>();
    const author = new Map<string, number>();
    const type = new Map<string, number>();
    const review = new Map<string, number>();
    const status = new Map<string, number>();
    let trackedCount = 0;
    let untrackedCount = 0;
    for (const it of tabItems) {
      repo.set(it.repo, (repo.get(it.repo) ?? 0) + 1);
      if (it.author_login) author.set(it.author_login, (author.get(it.author_login) ?? 0) + 1);
      type.set(it.type, (type.get(it.type) ?? 0) + 1);
      if (it.review_decision) review.set(it.review_decision, (review.get(it.review_decision) ?? 0) + 1);
      const sk = statusKey(it);
      status.set(sk, (status.get(sk) ?? 0) + 1);
      if (trackedIds.has(it.id)) trackedCount++;
      else untrackedCount++;
    }
    const byCount = (a: [string, number], b: [string, number]) => b[1] - a[1] || a[0].localeCompare(b[0]);
    const byReviewOrder = (a: [string, number], b: [string, number]) =>
      (REVIEW_ORDER[a[0]] ?? 99) - (REVIEW_ORDER[b[0]] ?? 99) || a[0].localeCompare(b[0]);
    return {
      repoEntries: [...repo.entries()].sort(byCount),
      authorEntries: [...author.entries()].sort(byCount),
      typeEntries: [...type.entries()].sort((a, b) => a[0].localeCompare(b[0])),
      reviewEntries: [...review.entries()].sort(byReviewOrder),
      statusEntries: [...status.entries()].sort(byCount),
      trackingEntries: [["tracked", trackedCount], ["untracked", untrackedCount]] as [string, number][],
    };
  }, [tabItems, trackedIds]);

  const effRepo = repoFilter && tabItems.some((i) => i.repo === repoFilter) ? repoFilter : null;
  const effAuthor = authorFilter && tabItems.some((i) => i.author_login === authorFilter) ? authorFilter : null;
  const effType = typeFilter && tabItems.some((i) => i.type === typeFilter) ? typeFilter : null;
  const effReview = reviewFilter && tabItems.some((i) => i.review_decision === reviewFilter) ? reviewFilter : null;
  const effStatus = statusFilter && tabItems.some((i) => statusKey(i) === statusFilter) ? statusFilter : null;

  const items = tabItems.filter((i) => {
    if (effRepo && i.repo !== effRepo) return false;
    if (effAuthor && i.author_login !== effAuthor) return false;
    if (effType && i.type !== effType) return false;
    if (effReview && i.review_decision !== effReview) return false;
    if (effStatus && statusKey(i) !== effStatus) return false;
    if (trackingFilter === "tracked" && !trackedIds.has(i.id)) return false;
    if (trackingFilter === "untracked" && trackedIds.has(i.id)) return false;
    return true;
  });

  const visibleIds = items.map((i) => i.id);
  useSyncScope("inbox", onHome, visibleIds);

  const busy = engage.isPending || unengage.isPending || untrack.isPending || openPr.isPending || track.isPending;
  const lastRefresh = metaQuery.data?.lastRefreshAt;

  return (
    <section className="cr-main">
      <header className="cr-pagehead">
        <div>
          <h1 className="cr-h1">Inbox</h1>
          {metaQuery.data?.viewerLogin && (
            <p className="cr-sub">
              Logged in as <span className="mono">@{metaQuery.data.viewerLogin}</span>
            </p>
          )}
        </div>
        <div className="cr-spacer" />
        <div className="inbox-head-actions">
          <div className="inbox-head-buttons">
            <SyncButton scope="inbox" sync={sync} visibleIds={visibleIds} />
          </div>
          {lastRefresh && <span className="cr-sub">synced {timeAgo(lastRefresh)}</span>}
        </div>
      </header>

      <div className="cr-tabs" role="tablist">
        {TABS.map((t) => (
          <button
            key={t.key}
            role="tab"
            aria-selected={t.key === active}
            className={`cr-tab${t.key === active ? " active" : ""}`}
            onClick={() => setActive(t.key)}
          >
            <Icon name={t.icon} size={13} />
            <span>{t.label}</span>
            <span className="n">{buckets[t.key].length}</span>
          </button>
        ))}
      </div>

      <div className="inbox-layout">
        <aside className="cr-rail">
          <FilterList title="Tracking" entries={trackingEntries} selected={trackingFilter} onSelect={setTrackingFilter} />
          <FilterList title="Status" entries={statusEntries} selected={effStatus} onSelect={setStatusFilter} />
          <FilterList title="Type" entries={typeEntries} selected={effType} onSelect={setTypeFilter} />
          <FilterList
            title="Review"
            entries={reviewEntries}
            selected={effReview}
            onSelect={setReviewFilter}
            renderLabel={(key) => <span className="lbl">{reviewLabel(key)}</span>}
          />
          <FilterList
            title="Repositories"
            entries={repoEntries}
            selected={effRepo}
            onSelect={setRepoFilter}
            renderLabel={(key) => <RepoName className="lbl mono" name={key} />}
          />
          <FilterList title="Users" entries={authorEntries} selected={effAuthor} onSelect={setAuthorFilter} />
        </aside>

        <div className="cr-list">
          {sync.isPending && (
            <div className="inbox-loading">
              <span className="spinner spinner-lg" />
              <span className="muted">Syncing from GitHub…</span>
            </div>
          )}
          {inboxQuery.isLoading && <p className="muted">Loading…</p>}
          {!inboxQuery.isLoading && items.length === 0 && (
            <p className="muted">
              Nothing here.{" "}
              {inboxItems.length === 0 && closedItems.length === 0 && "Hit Sync to fetch from GitHub."}
            </p>
          )}
          {items.map((item) => {
            const tab = TABS.find((t) => t.key === active)!;
            const existingReviewId = reviewByPr.get(`${item.repo}#${item.number}`);
            const canBoardTrack = item.type === "issue" || !isMine(item);
            return (
              <InboxItemRow
                key={item.id}
                item={item}
                variant={tab.variant}
                busy={busy}
                existingReviewId={existingReviewId}
                onEngage={() => engage.mutate(item.id)}
                onUnengage={() => unengage.mutate(item.id)}
                onUntrack={() => untrack.mutate(item.id)}
                onRetrack={() => {}}
                onOpenReview={() => openPr.mutate(item)}
                onOpenExisting={() => existingReviewId != null && openReview(existingReviewId)}
                boardTracked={trackedIds.has(item.id)}
                onBoardTrack={canBoardTrack ? () => track.mutate(item) : undefined}
              />
            );
          })}
        </div>
      </div>
    </section>
  );
}

function FilterList({
  title,
  entries,
  selected,
  onSelect,
  mono,
  renderLabel,
}: {
  title: string;
  entries: [string, number][];
  selected: string | null;
  onSelect: (v: string | null) => void;
  mono?: boolean;
  renderLabel?: (key: string) => React.ReactNode;
}) {
  if (entries.length === 0) return null;
  return (
    <div className="cr-rail-group">
      <div className="cr-rail-h">
        <span>{title}</span>
        {selected && (
          <button className="cr-rail-clear" onClick={() => onSelect(null)}>
            clear
          </button>
        )}
      </div>
      {entries.map(([key, count]) => (
        <button
          key={key}
          className={`cr-rail-item${key === selected ? " on" : ""}`}
          onClick={() => onSelect(key === selected ? null : key)}
        >
          {renderLabel ? renderLabel(key) : <span className={mono ? "lbl mono" : "lbl"}>{key}</span>}
          <span className="count">{count}</span>
        </button>
      ))}
    </div>
  );
}
