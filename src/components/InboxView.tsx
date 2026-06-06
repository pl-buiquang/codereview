import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { parseBotLogins, useSettingsStore } from "../lib/settings";
import { timeAgo } from "../lib/timeAgo";
import { useUIStore } from "../store";
import type { InboxItem } from "../lib/types";
import { InboxItemRow, type RowVariant } from "./InboxItemRow";

type BucketKey = "needs-you" | "authored" | "team-review" | "bots" | "visited" | "closed";

const TABS: { key: BucketKey; label: string; emoji: string; variant: RowVariant }[] = [
  { key: "needs-you", label: "Needs you", emoji: "🔥", variant: "inbox" },
  { key: "authored", label: "Authored", emoji: "📥", variant: "inbox" },
  { key: "team-review", label: "Team review", emoji: "👥", variant: "inbox" },
  { key: "bots", label: "Bots", emoji: "🤖", variant: "inbox" },
  { key: "visited", label: "Visited", emoji: "👁", variant: "visited" },
  { key: "closed", label: "Closed", emoji: "✅", variant: "closed" },
];

function isVisited(item: InboxItem): boolean {
  return !!item.engaged_at && item.engaged_at >= item.updated_at;
}

function bucketFor(item: InboxItem, bots: Set<string>): BucketKey {
  if (isVisited(item)) return "visited";
  if (item.author_login && bots.has(item.author_login.toLowerCase())) return "bots";
  const reasons = item.reasons.map((r) => r.reason);
  if (reasons.includes("mention") || reasons.includes("direct_review") || reasons.includes("assigned")) {
    return "needs-you";
  }
  if (reasons.includes("team_review")) return "team-review";
  return "authored";
}

export function InboxView() {
  const queryClient = useQueryClient();
  const openReview = useUIStore((s) => s.openReview);
  const botLogins = useSettingsStore((s) => s.botLogins);
  const [active, setActive] = useState<BucketKey>("needs-you");
  const [repoFilter, setRepoFilter] = useState<string | null>(null);
  const [authorFilter, setAuthorFilter] = useState<string | null>(null);
  const [typeFilter, setTypeFilter] = useState<string | null>(null);

  const inboxQuery = useQuery({ queryKey: ["inbox"], queryFn: api.listInbox });
  const closedQuery = useQuery({ queryKey: ["closed"], queryFn: api.listClosed });
  const metaQuery = useQuery({ queryKey: ["inbox-meta"], queryFn: api.inboxMeta });

  const invalidate = () => {
    queryClient.invalidateQueries({ queryKey: ["inbox"] });
    queryClient.invalidateQueries({ queryKey: ["closed"] });
    queryClient.invalidateQueries({ queryKey: ["archive"] });
    queryClient.invalidateQueries({ queryKey: ["inbox-meta"] });
  };

  const refresh = useMutation({
    mutationFn: api.refreshInbox,
    onSuccess: (r) => {
      invalidate();
      toast.success(`Refreshed: ${r.itemCount} items, ${r.closedCount} closed (${r.durationMs}ms)`);
    },
    onError: (e) => toast.error(`Refresh failed:\n${String(e)}`),
  });

  const onErr = (e: unknown) => toast.error(String(e));
  const engage = useMutation({ mutationFn: api.engageItem, onSuccess: invalidate, onError: onErr });
  const unengage = useMutation({ mutationFn: api.unengageItem, onSuccess: invalidate, onError: onErr });
  const untrack = useMutation({ mutationFn: api.untrackItem, onSuccess: invalidate, onError: onErr });

  const openPr = useMutation({
    mutationFn: (item: InboxItem) => {
      const [owner, name] = item.repo.split("/");
      return api.openPrReview(item.id, owner, name, item.number);
    },
    onSuccess: (review) => {
      invalidate();
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
  const { repoEntries, authorEntries, typeEntries } = useMemo(() => {
    const repo = new Map<string, number>();
    const author = new Map<string, number>();
    const type = new Map<string, number>();
    for (const it of tabItems) {
      repo.set(it.repo, (repo.get(it.repo) ?? 0) + 1);
      if (it.author_login) author.set(it.author_login, (author.get(it.author_login) ?? 0) + 1);
      type.set(it.type, (type.get(it.type) ?? 0) + 1);
    }
    const byCount = (a: [string, number], b: [string, number]) => b[1] - a[1] || a[0].localeCompare(b[0]);
    return {
      repoEntries: [...repo.entries()].sort(byCount),
      authorEntries: [...author.entries()].sort(byCount),
      typeEntries: [...type.entries()].sort((a, b) => a[0].localeCompare(b[0])),
    };
  }, [tabItems]);

  const effRepo = repoFilter && tabItems.some((i) => i.repo === repoFilter) ? repoFilter : null;
  const effAuthor = authorFilter && tabItems.some((i) => i.author_login === authorFilter) ? authorFilter : null;
  const effType = typeFilter && tabItems.some((i) => i.type === typeFilter) ? typeFilter : null;

  const items = tabItems.filter((i) => {
    if (effRepo && i.repo !== effRepo) return false;
    if (effAuthor && i.author_login !== effAuthor) return false;
    if (effType && i.type !== effType) return false;
    return true;
  });

  const busy = engage.isPending || unengage.isPending || untrack.isPending || openPr.isPending;
  const lastRefresh = metaQuery.data?.lastRefreshAt;

  return (
    <section className="main-panel inbox-panel">
      <header className="inbox-header">
        <div>
          <h2 className="inbox-h">Inbox</h2>
          {metaQuery.data?.viewerLogin && (
            <p className="muted small">
              Logged in as <span className="mono">@{metaQuery.data.viewerLogin}</span>
            </p>
          )}
        </div>
        <div className="inbox-header-actions">
          {lastRefresh && <span className="muted small">updated {timeAgo(lastRefresh)}</span>}
          <button className="btn-primary" onClick={() => refresh.mutate()} disabled={refresh.isPending}>
            {refresh.isPending ? (
              <>
                <span className="spinner" /> Refreshing…
              </>
            ) : (
              "↻ Refresh"
            )}
          </button>
        </div>
      </header>

      <div className="inbox-tabs" role="tablist">
        {TABS.map((t) => (
          <button
            key={t.key}
            role="tab"
            aria-selected={t.key === active}
            className={`inbox-tab${t.key === active ? " active" : ""}`}
            onClick={() => setActive(t.key)}
          >
            <span>{t.emoji}</span>
            <span>{t.label}</span>
            <span className="inbox-tab-count">{buckets[t.key].length}</span>
          </button>
        ))}
      </div>

      <div className="inbox-layout">
        <aside className="inbox-filters">
          <FilterList title="Type" entries={typeEntries} selected={effType} onSelect={setTypeFilter} />
          <FilterList title="Repositories" entries={repoEntries} selected={effRepo} onSelect={setRepoFilter} mono />
          <FilterList title="Users" entries={authorEntries} selected={effAuthor} onSelect={setAuthorFilter} />
        </aside>

        <div className="inbox-list">
          {refresh.isPending && (
            <div className="inbox-loading">
              <span className="spinner spinner-lg" />
              <span className="muted">Refreshing from GitHub…</span>
            </div>
          )}
          {inboxQuery.isLoading && <p className="muted">Loading…</p>}
          {!inboxQuery.isLoading && items.length === 0 && (
            <p className="muted">
              Nothing here.{" "}
              {inboxItems.length === 0 && closedItems.length === 0 && "Hit Refresh to fetch from GitHub."}
            </p>
          )}
          {items.map((item) => {
            const tab = TABS.find((t) => t.key === active)!;
            return (
              <InboxItemRow
                key={item.id}
                item={item}
                variant={tab.variant}
                busy={busy}
                onEngage={() => engage.mutate(item.id)}
                onUnengage={() => unengage.mutate(item.id)}
                onUntrack={() => untrack.mutate(item.id)}
                onRetrack={() => {}}
                onOpenReview={() => openPr.mutate(item)}
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
}: {
  title: string;
  entries: [string, number][];
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
        {entries.map(([key, count]) => (
          <li key={key}>
            <button
              className={`filter-item${key === selected ? " active" : ""}`}
              onClick={() => onSelect(key === selected ? null : key)}
            >
              <span className={mono ? "mono truncate" : "truncate"}>{key}</span>
              <span className="filter-count">{count}</span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
