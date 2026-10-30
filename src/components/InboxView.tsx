import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { parseBotLogins, parseRepoBasePaths, useSettingsStore } from "../lib/settings";
import { useInboxCounts } from "../lib/inboxCounts";
import { useSyncNow, useSyncScope } from "../lib/sync";
import { timeAgo } from "../lib/timeAgo";
import { useUIStore } from "../store";
import type { InboxItem } from "../lib/types";
import { approveAndAutoMerge } from "../lib/prActions";
import { InboxItemRow, type RowVariant } from "./InboxItemRow";
import { ConversationsView } from "./ConversationsView";
import { FilterList, FilterRail, MultiFilterList } from "./RailFilters";
import { QuickViewPane } from "./QuickViewPane";
import { RepoName } from "./RepoName";
import { Icon, type IconName } from "./icons";
import { SyncButton } from "./SyncButton";

type TabKey = "new" | "archive" | "conversations";

const TABS: { key: TabKey; label: string; icon: IconName; variant: RowVariant }[] = [
  { key: "new", label: "New", icon: "inbox", variant: "new" },
  { key: "conversations", label: "Conversations", icon: "comment", variant: "new" },
  { key: "archive", label: "Archive", icon: "archive", variant: "archive" },
];

const ARCHIVE_PAGE = 100;

/** Why an item is in the queue, coarsest first. */
type ReasonGroup = "needs-you" | "team-review" | "authored";

const REASON_GROUP_LABEL: Record<ReasonGroup, string> = {
  "needs-you": "needs you",
  "team-review": "team review",
  authored: "authored",
};

const CAUSES: { key: string; label: string }[] = [
  { key: "dismissed", label: "dismissed" },
  { key: "closed", label: "closed" },
  { key: "discarded", label: "discarded" },
  { key: "tracked", label: "tracked" },
  { key: "forgotten", label: "forgotten" },
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

function reasonGroup(item: InboxItem): ReasonGroup {
  const reasons = item.reasons.map((r) => r.reason);
  if (reasons.includes("mention") || reasons.includes("direct_review") || reasons.includes("assigned")) {
    return "needs-you";
  }
  // Authored wins over team_review: a team you're on may also be requested on your own item.
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
    repos: repoFilter,
    authors: authorFilter,
    type: typeFilter,
    review: reviewFilter,
    status: statusFilter,
    reason: reasonFilter,
    flag: flagFilter,
    cause: causeFilter,
    unread: unreadFilter,
  } = filters;
  const active: TabKey =
    filters.bucket === "archive" ? "archive" : filters.bucket === "conversations" ? "conversations" : "new";
  const setActive = (bucket: TabKey) => setInboxFilters({ bucket });
  const setRepoFilter = (repos: string[]) => setInboxFilters({ repos });
  const setAuthorFilter = (authors: string[]) => setInboxFilters({ authors });
  const setTypeFilter = (type: string | null) => setInboxFilters({ type });
  const setReviewFilter = (review: string | null) => setInboxFilters({ review });
  const setStatusFilter = (status: string | null) => setInboxFilters({ status });
  const setReasonFilter = (reason: string | null) => setInboxFilters({ reason });
  const setFlagFilter = (flag: string | null) => setInboxFilters({ flag });
  const setCauseFilter = (cause: string | null) => setInboxFilters({ cause });
  const setUnreadFilter = (on: string | null) => setInboxFilters({ unread: on != null });

  const [search, setSearch] = useState("");
  const [archiveLimit, setArchiveLimit] = useState(ARCHIVE_PAGE);
  const searchTerm = search.trim();

  const inboxQuery = useQuery({ queryKey: ["inbox"], queryFn: api.listInbox });
  const archiveQuery = useQuery({
    queryKey: ["inbox-archive", searchTerm, causeFilter, archiveLimit],
    queryFn: () => api.listInboxArchive(searchTerm || null, causeFilter, archiveLimit),
    enabled: active === "archive",
  });
  const metaQuery = useQuery({ queryKey: ["inbox-meta"], queryFn: api.inboxMeta });
  const { newItems: unreadNew, unreadConversations } = useInboxCounts();

  const reviewsQuery = useQuery({ queryKey: ["reviews", null], queryFn: () => api.listReviews(null) });
  const trackedQuery = useQuery({ queryKey: ["board-tracked"], queryFn: api.boardTrackedIds });
  const trackedIds = useMemo(() => new Set(trackedQuery.data ?? []), [trackedQuery.data]);

  // Latest unpublished review per GitHub PR, keyed "owner/name#number" like
  // InboxItem.repo. list_reviews is newest-first, so the first id seen per key
  // is the most recent one.
  const reviewByPr = useMemo(() => {
    // Unpublished reviews first, so a published one is only reopened when it is the PR's only review.
    const m = new Map<string, number>();
    const published = new Map<string, number>();
    for (const s of reviewsQuery.data ?? []) {
      if (s.target.kind !== "github_pr" || s.target.github_pr_number == null || !s.repo_remote) continue;
      const key = `${s.repo_remote}#${s.target.github_pr_number}`;
      const into = s.review.status === "published" ? published : m;
      if (!into.has(key)) into.set(key, s.review.id);
    }
    for (const [key, id] of published) if (!m.has(key)) m.set(key, id);
    return m;
  }, [reviewsQuery.data]);

  const invalidate = () => {
    queryClient.invalidateQueries({ queryKey: ["inbox"] });
    queryClient.invalidateQueries({ queryKey: ["inbox-archive"] });
    queryClient.invalidateQueries({ queryKey: ["inbox-meta"] });
  };

  const sync = useSyncNow("inbox");

  const onErr = (e: unknown) => toast.error(String(e));
  const archive = useMutation({ mutationFn: api.archiveItem, onSuccess: invalidate, onError: onErr });
  const forget = useMutation({ mutationFn: api.forgetItem, onSuccess: invalidate, onError: onErr });
  const restore = useMutation({ mutationFn: api.restoreItem, onSuccess: invalidate, onError: onErr });
  const markViewed = useMutation({ mutationFn: api.markItemViewed, onSuccess: invalidate, onError: onErr });
  const markUnread = useMutation({ mutationFn: api.markItemUnread, onSuccess: invalidate, onError: onErr });
  const track = useMutation({
    mutationFn: (item: InboxItem) => api.boardTrack(item.url),
    onSuccess: () => {
      invalidate();
      queryClient.invalidateQueries({ queryKey: ["board-tracked"] });
      queryClient.invalidateQueries({ queryKey: ["board"] });
      toast.info("Tracked on the board");
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
      queryClient.invalidateQueries({ queryKey: ["board-tracked"] });
      queryClient.invalidateQueries({ queryKey: ["board"] });
      openReview(review.id);
    },
    onError: (e) => toast.error(`Could not open review:\n${String(e)}`),
  });

  const [quickItem, setQuickItem] = useState<InboxItem | null>(null);
  const closeQuick = () => setQuickItem(null);

  // Quick view: Open review resumes the latest review when there is one, and either way
  // the PR ends up tracked, which takes it out of the inbox.
  const quickOpenReview = (item: InboxItem) => {
    closeQuick();
    const existing = reviewByPr.get(`${item.repo}#${item.number}`);
    if (existing == null) {
      openPr.mutate(item);
      return;
    }
    api
      .boardTrack(item.url)
      .catch(() => {})
      .finally(() => {
        invalidate();
        queryClient.invalidateQueries({ queryKey: ["board-tracked"] });
        queryClient.invalidateQueries({ queryKey: ["board"] });
      });
    openReview(existing);
  };

  // Quick view: approve in the background. The PR is tracked up front so it leaves the
  // inbox at once; the toast says whether GitHub accepted it.
  const quickApprove = (item: InboxItem, merge: boolean, autoMergeOn: boolean) => {
    closeQuick();
    const [owner, name] = item.repo.split("/");
    const label = `#${item.number}`;
    api
      .boardTrack(item.url)
      .catch(() => {})
      .finally(() => {
        invalidate();
        queryClient.invalidateQueries({ queryKey: ["board-tracked"] });
        queryClient.invalidateQueries({ queryKey: ["board"] });
      });
    const run = merge ? approveAndAutoMerge(owner, name, item.number, autoMergeOn) : api.approvePr(owner, name, item.number);
    run
      .then(() => {
        toast.success(merge ? `Approved ${label}, auto-merge enabled` : `Approved ${label}`);
        queryClient.invalidateQueries({ queryKey: ["board"] });
        queryClient.invalidateQueries({ queryKey: ["mirror-pr-meta", owner, name, item.number] });
      })
      .catch((e) => toast.error(`${merge ? "Approve & merge" : "Approve"} ${label} failed:\n${String(e)}`));
  };

  const botSet = useMemo(() => parseBotLogins(botLogins), [botLogins]);
  const isBot = (item: InboxItem) =>
    item.author_is_bot || (!!item.author_login && botSet.has(item.author_login.toLowerCase()));

  const newItems = inboxQuery.data ?? [];
  const archiveItems = archiveQuery.data ?? [];
  const tabItems = active === "archive" ? archiveItems : newItems;

  // Build sidebar filter counts from the active tab's items.
  const {
    repoEntries,
    authorEntries,
    typeEntries,
    reviewEntries,
    statusEntries,
    reasonEntries,
    flagEntries,
    unreadEntries,
  } = useMemo(() => {
      const repo = new Map<string, number>();
      const author = new Map<string, number>();
      const type = new Map<string, number>();
      const review = new Map<string, number>();
      const status = new Map<string, number>();
      const reason = new Map<string, number>();
      const flag = new Map<string, number>();
      let unread = 0;
      const bump = (m: Map<string, number>, k: string) => m.set(k, (m.get(k) ?? 0) + 1);
      for (const it of tabItems) {
        bump(repo, it.repo);
        if (it.author_login) bump(author, it.author_login);
        bump(type, it.type);
        if (it.review_decision) bump(review, it.review_decision);
        bump(status, statusKey(it));
        bump(reason, reasonGroup(it));
        bump(flag, isBot(it) ? "bot" : "human");
        if (it.stale) bump(flag, "stale");
        if (active === "new" && !it.viewed_at) unread++;
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
        reasonEntries: [...reason.entries()].sort(byCount),
        flagEntries: [...flag.entries()].sort(byCount),
        unreadEntries: (unread > 0 ? [["unread", unread]] : []) as [string, number][],
      };
      // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [tabItems, botSet, active]);

  const effRepos = repoFilter.filter((r) => tabItems.some((i) => i.repo === r));
  const effAuthors = authorFilter.filter((a) => tabItems.some((i) => i.author_login === a));
  const effType = typeFilter && tabItems.some((i) => i.type === typeFilter) ? typeFilter : null;
  const effReview = reviewFilter && tabItems.some((i) => i.review_decision === reviewFilter) ? reviewFilter : null;
  const effStatus = statusFilter && tabItems.some((i) => statusKey(i) === statusFilter) ? statusFilter : null;
  const effReason = reasonFilter && tabItems.some((i) => reasonGroup(i) === reasonFilter) ? reasonFilter : null;
  const effFlag =
    flagFilter &&
    tabItems.some((i) => (flagFilter === "stale" ? i.stale : flagFilter === "bot" ? isBot(i) : !isBot(i)))
      ? flagFilter
      : null;

  const effUnread = unreadFilter && active === "new" && tabItems.some((i) => !i.viewed_at);

  const items = tabItems.filter((i) => {
    if (effUnread && i.viewed_at) return false;
    if (effRepos.length > 0 && !effRepos.includes(i.repo)) return false;
    if (effAuthors.length > 0 && !effAuthors.includes(i.author_login ?? "")) return false;
    if (effType && i.type !== effType) return false;
    if (effReview && i.review_decision !== effReview) return false;
    if (effStatus && statusKey(i) !== effStatus) return false;
    if (effReason && reasonGroup(i) !== effReason) return false;
    if (effFlag === "stale" && !i.stale) return false;
    if (effFlag === "bot" && !isBot(i)) return false;
    if (effFlag === "human" && isBot(i)) return false;
    return true;
  });

  const visibleIds = items.map((i) => i.id);
  useSyncScope("inbox", onHome, visibleIds);

  const busy = archive.isPending || forget.isPending || restore.isPending || markViewed.isPending || markUnread.isPending || openPr.isPending || track.isPending;
  const lastRefresh = metaQuery.data?.lastRefreshAt;
  const loading = active === "archive" ? archiveQuery.isLoading : inboxQuery.isLoading;
  const tab = TABS.find((t) => t.key === active)!;
  const canLoadMore = active === "archive" && archiveItems.length >= archiveLimit;

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
            {t.key === "new" && unreadNew > 0 && <span className="n">{unreadNew}</span>}
            {t.key === "conversations" && unreadConversations > 0 && <span className="n">{unreadConversations}</span>}
          </button>
        ))}
      </div>

      {active === "conversations" ? (
        <ConversationsView />
      ) : (
      <div className="inbox-layout">
        <FilterRail id="inbox">
          <FilterList
            title="Show"
            entries={unreadEntries}
            selected={effUnread ? "unread" : null}
            onSelect={setUnreadFilter}
          />
          <FilterList
            title="Reason"
            entries={reasonEntries}
            selected={effReason}
            onSelect={setReasonFilter}
            renderLabel={(key) => <span className="lbl">{REASON_GROUP_LABEL[key as ReasonGroup] ?? key}</span>}
          />
          <FilterList title="Author" entries={flagEntries} selected={effFlag} onSelect={setFlagFilter} />
          <FilterList title="Status" entries={statusEntries} selected={effStatus} onSelect={setStatusFilter} />
          <FilterList title="Type" entries={typeEntries} selected={effType} onSelect={setTypeFilter} />
          <FilterList
            title="Review"
            entries={reviewEntries}
            selected={effReview}
            onSelect={setReviewFilter}
            renderLabel={(key) => <span className="lbl">{reviewLabel(key)}</span>}
          />
          <MultiFilterList
            title="Repositories"
            entries={repoEntries}
            selected={effRepos}
            onChange={setRepoFilter}
            renderLabel={(key) => <RepoName className="lbl ident" name={key} />}
          />
          <MultiFilterList title="Users" entries={authorEntries} selected={effAuthors} onChange={setAuthorFilter} />
        </FilterRail>

        <div className="cr-list">
          {active === "archive" && (
            <div className="inbox-toolbar">
              <input
                className="input archive-search"
                placeholder="Search archive by title, repo, or author…"
                value={search}
                onChange={(e) => {
                  setSearch(e.target.value);
                  setArchiveLimit(ARCHIVE_PAGE);
                }}
              />
              {CAUSES.map((c) => (
                <button
                  key={c.key}
                  className={`chip${causeFilter === c.key ? " accent" : ""}`}
                  aria-pressed={causeFilter === c.key}
                  onClick={() => {
                    setCauseFilter(causeFilter === c.key ? null : c.key);
                    setArchiveLimit(ARCHIVE_PAGE);
                  }}
                >
                  {c.label}
                </button>
              ))}
            </div>
          )}
          {sync.isPending && (
            <div className="inbox-loading">
              <span className="spinner spinner-lg" />
              <span className="muted">Syncing from GitHub…</span>
            </div>
          )}
          {loading && <p className="muted">Loading…</p>}
          {!loading && items.length === 0 && (
            <p className="muted">
              {active === "archive"
                ? searchTerm || causeFilter
                  ? "No matches."
                  : "Nothing archived."
                : newItems.length === 0
                  ? "Inbox zero. Hit Sync to fetch from GitHub."
                  : "Nothing matches these filters."}
            </p>
          )}
          {items.map((item) => {
            const existingReviewId = reviewByPr.get(`${item.repo}#${item.number}`);
            return (
              <InboxItemRow
                key={item.id}
                item={item}
                variant={tab.variant}
                busy={busy}
                isBot={isBot(item)}
                existingReviewId={existingReviewId}
                onArchive={() => archive.mutate(item.id)}
                onForget={() => forget.mutate(item.id)}
                onRestore={() => restore.mutate(item.id)}
                onMarkViewed={() => markViewed.mutate(item.id)}
                onMarkUnread={() => markUnread.mutate(item.id)}
                onOpenReview={() => openPr.mutate(item)}
                onOpenExisting={() => {
                  if (existingReviewId == null) return;
                  if (!item.viewed_at) markViewed.mutate(item.id);
                  openReview(existingReviewId);
                }}
                onQuickView={() => setQuickItem(item)}
                boardTracked={trackedIds.has(item.id)}
                onBoardTrack={() => track.mutate(item)}
              />
            );
          })}
          {canLoadMore && (
            <button className="btn" onClick={() => setArchiveLimit((n) => n + ARCHIVE_PAGE)}>
              Load more
            </button>
          )}
        </div>
      </div>
      )}
      {quickItem && (
        <QuickViewPane
          item={quickItem}
          viewerLogin={metaQuery.data?.viewerLogin ?? null}
          onClose={closeQuick}
          onOpenReview={() => quickOpenReview(quickItem)}
          onApprove={() => quickApprove(quickItem, false, false)}
          onApproveAndMerge={(on) => quickApprove(quickItem, true, on)}
        />
      )}
    </section>
  );
}
