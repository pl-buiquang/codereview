import { useEffect, useMemo, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { timeAgo } from "../lib/timeAgo";
import { confirmDialog } from "../lib/confirm";
import { BOARD_POLL_OPTIONS, parseRepoBasePaths, useSettingsStore } from "../lib/settings";
import { findRepoByFullName } from "../lib/repoLabel";
import { useUIStore } from "../store";
import type { BoardCard as BoardCardT, BoardLane, BoardMetaKind, ClaudeSession } from "../lib/types";
import { BoardCard, cardTags, ciStatus, prStatus, prStatusLabel } from "./BoardCard";
import { RepoName } from "./RepoName";
import { TagFilter } from "./TagFilter";
import { TrackPrDialog } from "./TrackPrDialog";
import { Icon } from "./icons";

const LANES: { key: BoardLane; label: string }[] = [
  { key: "backlog", label: "Backlog" },
  { key: "in_progress", label: "In progress" },
  { key: "submitted", label: "Submitted" },
  { key: "reviews", label: "Reviews" },
  { key: "done", label: "Done" },
];

const STATUS_ORDER: Record<string, number> = {
  review_required: 0,
  changes_requested: 1,
  commented: 2,
  approved: 3,
  draft: 4,
  ready: 5,
};

const CI_ORDER: Record<string, number> = {
  failure: 0,
  error: 1,
  pending: 2,
  expected: 3,
  success: 4,
};

function invalidateBoard(queryClient: ReturnType<typeof useQueryClient>) {
  queryClient.invalidateQueries({ queryKey: ["board"] });
}

export function BoardView() {
  const queryClient = useQueryClient();
  const openReviewTab = useUIStore((s) => s.openReview);
  const openRepo = useUIStore((s) => s.openRepo);
  const collapsedLanes = useUIStore((s) => s.collapsedLanes);
  const toggleLane = useUIStore((s) => s.toggleLane);
  const repoBasePaths = useSettingsStore((s) => s.repoBasePaths);
  const boardPollMs = useSettingsStore((s) => s.boardPollMs);
  const defaultTerminal = useSettingsStore((s) => s.defaultTerminal);
  const tmuxTarget = useSettingsStore((s) => s.tmuxTarget);
  const tmuxSessionName = useSettingsStore((s) => s.tmuxSessionName);
  const setBoardPollMs = useSettingsStore((s) => s.setBoardPollMs);
  const [repoFilter, setRepoFilter] = useState<string | null>(null);
  const [statusFilter, setStatusFilter] = useState<string | null>(null);
  const [ciFilter, setCiFilter] = useState<string | null>(null);
  const [tagFilter, setTagFilter] = useState<string[]>([]);
  const [tagExclude, setTagExclude] = useState<string[]>([]);
  const [addingLane, setAddingLane] = useState<BoardLane | null>(null);
  const [addValue, setAddValue] = useState("");
  const [showTrackDialog, setShowTrackDialog] = useState(false);

  const boardQuery = useQuery({ queryKey: ["board"], queryFn: api.boardList });
  const tagsQuery = useQuery({ queryKey: ["board-meta-tags"], queryFn: api.boardMetaTags });
  const repositoriesQuery = useQuery({ queryKey: ["repositories"], queryFn: api.listRepositories });
  const lastRefreshQuery = useQuery({ queryKey: ["board-last-refresh"], queryFn: api.boardLastRefreshAt });
  const lastRefresh = lastRefreshQuery.data;
  const liveSessionsQuery = useQuery({
    queryKey: ["claude-sessions", 0],
    queryFn: () => api.listClaudeSessions(0, ""),
    refetchInterval: 15_000,
  });
  const cardSessionsQuery = useQuery({
    queryKey: ["board-card-sessions"],
    queryFn: api.boardCardSessions,
    refetchInterval: 15_000,
  });
  const focusableByCard = useMemo(() => {
    const byId = new Map((liveSessionsQuery.data ?? []).map((s) => [s.session_id, s]));
    const out = new Map<string, ClaudeSession[]>();
    for (const link of cardSessionsQuery.data ?? []) {
      const s = byId.get(link.session_id);
      if (!s || (s.focus !== "ready" && s.agent_view?.focus !== "ready")) continue;
      out.set(link.card_id, [...(out.get(link.card_id) ?? []), s]);
    }
    return out;
  }, [liveSessionsQuery.data, cardSessionsQuery.data]);

  const openCardRepo = (repo: string) => {
    const match = findRepoByFullName(repositoriesQuery.data ?? [], repo);
    if (match) openRepo(match.id);
    else {
      api.openUrl(`https://github.com/${repo}`).catch(onErr);
      toast.success("Repository not added locally — opened on GitHub instead");
    }
  };

  const invalidate = () => invalidateBoard(queryClient);
  const onErr = (e: unknown) => toast.error(String(e));

  const refresh = useMutation({
    mutationFn: (_opts?: { silent?: boolean }) => api.boardRefresh(),
    onSuccess: (r, opts) => {
      invalidate();
      queryClient.invalidateQueries({ queryKey: ["board-last-refresh"] });
      if (!opts?.silent)
        toast.success(`Refreshed board: ${r.cardCount} cards, ${r.autoTracked} newly tracked (${r.durationMs}ms)`);
    },
    onError: (e) => toast.error(`Board refresh failed:\n${String(e)}`),
  });

  const refreshRef = useRef(refresh);
  useEffect(() => {
    refreshRef.current = refresh;
  });
  useEffect(() => {
    if (boardPollMs <= 0) return;
    const id = setInterval(() => {
      if (document.visibilityState !== "visible") return;
      if (refreshRef.current.isPending) return;
      refreshRef.current.mutate({ silent: true });
    }, boardPollMs);
    return () => clearInterval(id);
  }, [boardPollMs]);

  const track = useMutation({ mutationFn: (reference: string) => api.boardTrack(reference), onSuccess: invalidate, onError: onErr });
  const discard = useMutation({ mutationFn: (cardId: string) => api.boardDiscard(cardId), onSuccess: invalidate, onError: onErr });
  const prMerge = useMutation({
    mutationFn: (cardId: string) => api.boardPrMerge(cardId),
    onSuccess: () => {
      invalidate();
      toast.success("PR squash-merged");
    },
    onError: (e) => toast.error(`Merge failed:\n${String(e)}`),
  });
  const prSetDraft = useMutation({
    mutationFn: ({ cardId, draft }: { cardId: string; draft: boolean }) => api.boardPrSetDraft(cardId, draft),
    onSuccess: (_, { draft }) => {
      invalidate();
      toast.success(draft ? "Converted to draft" : "Marked ready for review");
    },
    onError: onErr,
  });
  const prUpdateBranch = useMutation({
    mutationFn: ({ cardId, rebase }: { cardId: string; rebase: boolean }) => api.boardPrUpdateBranch(cardId, rebase),
    onSuccess: (_, { rebase }) => {
      invalidate();
      toast.success(rebase ? "Branch rebased" : "Branch updated");
    },
    onError: (e) => toast.error(`Update branch failed:\n${String(e)}`),
  });
  const prActionCardId =
    (prMerge.isPending && prMerge.variables) ||
    (prSetDraft.isPending && prSetDraft.variables?.cardId) ||
    (prUpdateBranch.isPending && prUpdateBranch.variables?.cardId) ||
    null;
  const linkPr = useMutation({
    mutationFn: ({ issueId, reference }: { issueId: string; reference: string }) => api.boardLinkPr(issueId, reference),
    onSuccess: invalidate,
    onError: onErr,
  });
  const unlink = useMutation({
    mutationFn: ({ issueId, prId }: { issueId: string; prId: string }) => api.boardUnlink(issueId, prId),
    onSuccess: invalidate,
    onError: onErr,
  });
  const archiveDone = useMutation({
    mutationFn: () => api.boardArchiveDone(30),
    onSuccess: (n) => {
      invalidate();
      toast.success(n === 1 ? "Archived 1 card" : `Archived ${n} cards`);
    },
    onError: onErr,
  });
  const openReview = useMutation({
    mutationFn: (cardId: string) => api.boardOpenReview(cardId, parseRepoBasePaths(repoBasePaths)),
    onSuccess: (review) => {
      invalidate();
      queryClient.invalidateQueries({ queryKey: ["reviews"] });
      openReviewTab(review.id);
    },
    onError: (e) => toast.error(`Could not open review:\n${String(e)}`),
  });
  const openVscode = useMutation({
    mutationFn: (cardId: string) => api.boardOpenVscode(cardId, parseRepoBasePaths(repoBasePaths)),
    onSuccess: invalidate,
    onError: (e) => toast.error(`Could not open VSCode:\n${String(e)}`),
  });
  const focusSession = useMutation({
    mutationFn: api.focusClaudeSession,
    onError: (e) => toast.error(`Could not focus terminal:\n${String(e)}`),
  });
  const openShell = useMutation({
    mutationFn: (cardId: string) => api.boardOpenShell(cardId, defaultTerminal, tmuxTarget, tmuxSessionName, parseRepoBasePaths(repoBasePaths)),
    onSuccess: invalidate,
    onError: (e) => toast.error(`Could not open terminal:\n${String(e)}`),
  });
  const openAgent = useMutation({
    mutationFn: (cardId: string) => api.boardOpenAgent(cardId, defaultTerminal, tmuxTarget, tmuxSessionName, parseRepoBasePaths(repoBasePaths)),
    onSuccess: () => {
      invalidate();
      queryClient.invalidateQueries({ queryKey: ["claude-sessions"] });
      queryClient.invalidateQueries({ queryKey: ["board-card-sessions"] });
    },
    onError: (e) => toast.error(`Could not open agent session:\n${String(e)}`),
  });
  const metaAdd = useMutation({
    mutationFn: (args: { cardId: string; kind: BoardMetaKind; value: string; tag: string | null; label: string | null }) =>
      api.boardMetaAdd(args),
    onSuccess: () => {
      invalidate();
      queryClient.invalidateQueries({ queryKey: ["board-meta-tags"] });
    },
    onError: onErr,
  });
  const metaDelete = useMutation({ mutationFn: (id: number) => api.boardMetaDelete(id), onSuccess: invalidate, onError: onErr });

  const allCards = useMemo(() => (boardQuery.data ?? []).filter((c) => c.lane != null), [boardQuery.data]);

  const { repoEntries, statusEntries, ciEntries, tagEntries } = useMemo(() => {
    const repo = new Map<string, number>();
    const status = new Map<string, number>();
    const ci = new Map<string, number>();
    const tag = new Map<string, number>();
    for (const c of allCards) {
      repo.set(c.repo, (repo.get(c.repo) ?? 0) + 1);
      const s = prStatus(c);
      if (s) status.set(s, (status.get(s) ?? 0) + 1);
      const ciS = ciStatus(c);
      if (ciS) ci.set(ciS, (ci.get(ciS) ?? 0) + 1);
      for (const t of cardTags(c)) tag.set(t, (tag.get(t) ?? 0) + 1);
    }
    const byCount = (a: [string, number], b: [string, number]) => b[1] - a[1] || a[0].localeCompare(b[0]);
    const byStatusOrder = (a: [string, number], b: [string, number]) =>
      (STATUS_ORDER[a[0]] ?? 99) - (STATUS_ORDER[b[0]] ?? 99);
    const byCiOrder = (a: [string, number], b: [string, number]) =>
      (CI_ORDER[a[0]] ?? 99) - (CI_ORDER[b[0]] ?? 99);
    return {
      repoEntries: [...repo.entries()].sort(byCount),
      statusEntries: [...status.entries()].sort(byStatusOrder),
      ciEntries: [...ci.entries()].sort(byCiOrder),
      tagEntries: [...tag.entries()].sort(byCount),
    };
  }, [allCards]);

  const effRepo = repoFilter && allCards.some((c) => c.repo === repoFilter) ? repoFilter : null;
  const effStatus = statusFilter && allCards.some((c) => prStatus(c) === statusFilter) ? statusFilter : null;
  const effCi = ciFilter && allCards.some((c) => ciStatus(c) === ciFilter) ? ciFilter : null;
  const availableTags = new Set(tagEntries.map(([t]) => t));
  const effTags = tagFilter.filter((t) => availableTags.has(t));
  const effExcludeTags = tagExclude.filter((t) => availableTags.has(t));

  const matches = (c: BoardCardT, skip?: "repo" | "status" | "ci" | "tags") => {
    if (skip !== "repo" && effRepo && c.repo !== effRepo) return false;
    if (skip !== "status" && effStatus && c.type === "pr" && prStatus(c) !== effStatus) return false;
    if (skip !== "ci" && effCi && ciStatus(c) !== effCi) return false;
    if (skip !== "tags" && effTags.length > 0 && !cardTags(c).some((t) => effTags.includes(t))) return false;
    if (effExcludeTags.length > 0 && cardTags(c).some((t) => effExcludeTags.includes(t))) return false;
    return true;
  };
  const filtered = allCards.filter((c) => matches(c));

  // Each facet counts cards matching every other active filter, so its own options stay comparable.
  const facetCounts = (skip: "repo" | "status" | "ci" | "tags", keysOf: (c: BoardCardT) => (string | null | undefined)[]) => {
    const m = new Map<string, number>();
    for (const c of allCards) {
      if (!matches(c, skip)) continue;
      for (const k of keysOf(c)) if (k) m.set(k, (m.get(k) ?? 0) + 1);
    }
    return m;
  };
  const withCounts = (entries: [string, number][], counts: Map<string, number>): [string, number][] =>
    entries.map(([k]) => [k, counts.get(k) ?? 0]);
  const repoShown = withCounts(repoEntries, facetCounts("repo", (c) => [c.repo]));
  const statusShown = withCounts(statusEntries, facetCounts("status", (c) => [prStatus(c)]));
  const ciShown = withCounts(ciEntries, facetCounts("ci", (c) => [ciStatus(c)]));
  const tagsShown = withCounts(tagEntries, facetCounts("tags", cardTags));

  const byLane = useMemo(() => {
    const m = new Map<BoardLane, BoardCardT[]>();
    for (const lane of LANES) m.set(lane.key, []);
    for (const c of filtered) {
      if (c.lane) m.get(c.lane as BoardLane)?.push(c);
    }
    for (const list of m.values()) list.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
    return m;
  }, [filtered]);

  const busy = track.isPending || discard.isPending || linkPr.isPending || unlink.isPending || metaAdd.isPending || metaDelete.isPending;
  const tags = tagsQuery.data ?? [];

  const submitAdd = (e: React.FormEvent) => {
    e.preventDefault();
    const v = addValue.trim();
    if (!v) return;
    track.mutate(v, { onSuccess: () => setAddValue("") });
    setAddingLane(null);
  };

  const confirmArchive = async () => {
    if (
      await confirmDialog({
        title: "Archive done cards",
        message: "Permanently archive Done cards whose PR closed or merged more than a month ago?",
        confirmLabel: "Archive",
      })
    ) {
      archiveDone.mutate();
    }
  };

  return (
    <section className="cr-main">
      <header className="cr-pagehead">
        <div>
          <h1 className="cr-h1">Board</h1>
        </div>
        <div className="cr-spacer" />
        <div className="inbox-head-actions">
          <div className="inbox-head-buttons">
            <button className="btn" onClick={() => setShowTrackDialog(true)}>
              <Icon name="plus" size={13} /> Track
            </button>
            <label className="sort-control" title="Automatically refresh the board on this interval">
              auto
              <select className="sort-select" value={boardPollMs} onChange={(e) => setBoardPollMs(Number(e.target.value))}>
                {BOARD_POLL_OPTIONS.map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
            </label>
            <button className="btn btn-primary" onClick={() => refresh.mutate({})} disabled={refresh.isPending}>
              {refresh.isPending ? (
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
          {lastRefresh && <span className="cr-sub">updated {timeAgo(lastRefresh)}</span>}
        </div>
      </header>

      <div className="inbox-layout">
        <aside className="cr-rail">
          <FilterList
            title="Status"
            entries={statusShown}
            selected={effStatus}
            onSelect={setStatusFilter}
            renderLabel={(k) => <span className="lbl">{prStatusLabel(k)}</span>}
          />
          <FilterList
            title="CI"
            entries={ciShown}
            selected={effCi}
            onSelect={setCiFilter}
            renderLabel={(k) => <span className="lbl">{prStatusLabel(k)}</span>}
          />
          <FilterList
            title="Repositories"
            entries={repoShown}
            selected={effRepo}
            onSelect={setRepoFilter}
            renderLabel={(k) => <RepoName className="lbl mono" name={k} />}
          />
          <TagFilter tags={tagsShown} selected={effTags} onChange={setTagFilter} unavailable={effExcludeTags} />
          <TagFilter
            tags={tagsShown}
            selected={effExcludeTags}
            onChange={setTagExclude}
            unavailable={effTags}
            title="Exclude tags"
            placeholder="Exclude tag…"
            variant="exclude"
          />
        </aside>

        <div className="board-lanes">
          {LANES.map((lane) => {
            const cards = byLane.get(lane.key) ?? [];
            const canAdd = lane.key === "backlog" || lane.key === "reviews";
            const collapsed = collapsedLanes.includes(lane.key);

            if (collapsed) {
              return (
                <button
                  className="board-lane board-lane-collapsed"
                  key={lane.key}
                  onClick={() => toggleLane(lane.key)}
                  title={`Expand ${lane.label}`}
                >
                  <span className="board-lane-collapsed-title">{lane.label}</span>
                  <span className="n">{cards.length}</span>
                </button>
              );
            }

            return (
              <div className="board-lane" key={lane.key}>
                <div className="board-lane-head">
                  <button className="btn-icon" title={`Collapse ${lane.label}`} onClick={() => toggleLane(lane.key)}>
                    <Icon name="chev" size={12} className="board-lane-collapse-chev" />
                  </button>
                  <span className="board-lane-title">{lane.label}</span>
                  <span className="n">{cards.length}</span>
                  <div className="cr-spacer" />
                  {canAdd && (
                    <button className="btn-icon" title="Track an issue/PR by URL" onClick={() => setAddingLane(addingLane === lane.key ? null : lane.key)}>
                      <Icon name="plus" size={12} />
                    </button>
                  )}
                  {lane.key === "done" && cards.length > 0 && (
                    <button className="btn-icon" title="Archive cards older than 1 month" onClick={confirmArchive}>
                      <Icon name="archive" size={12} />
                    </button>
                  )}
                </div>
                {addingLane === lane.key && (
                  <form className="board-lane-add" onSubmit={submitAdd}>
                    <input
                      className="input"
                      autoFocus
                      placeholder="GitHub URL or owner/repo#n"
                      value={addValue}
                      onChange={(e) => setAddValue(e.target.value)}
                    />
                    <button type="submit" className="btn btn-sm">
                      Track
                    </button>
                  </form>
                )}
                <div className="board-lane-cards">
                  {boardQuery.isLoading && <p className="muted">Loading…</p>}
                  {!boardQuery.isLoading && cards.length === 0 && <p className="muted faint">Empty</p>}
                  {cards.map((card) => (
                    <BoardCard
                      key={card.id}
                      card={card}
                      tags={tags}
                      busy={busy}
                      reviewBusy={openReview.isPending}
                      vscodeBusy={openVscode.isPending}
                      onOpenGh={() => api.openUrl(card.url).catch(onErr)}
                      onOpenRepo={() => openCardRepo(card.repo)}
                      onDiscard={card.is_mine ? null : () => discard.mutate(card.id)}
                      onLinkPr={card.type === "issue" ? (reference) => linkPr.mutate({ issueId: card.id, reference }) : null}
                      onUnlink={card.type === "issue" ? (prId) => unlink.mutate({ issueId: card.id, prId }) : null}
                      onOpenReview={card.type === "pr" && lane.key !== "backlog" ? () => openReview.mutate(card.id) : null}
                      onOpenVscode={card.type === "pr" && lane.key !== "backlog" ? () => openVscode.mutate(card.id) : null}
                      terminal={
                        card.type === "pr" && lane.key !== "backlog"
                          ? {
                              sessions: focusableByCard.get(card.id) ?? [],
                              busy: openShell.isPending || openAgent.isPending || focusSession.isPending,
                              onFocus: (id) => focusSession.mutate(id),
                              onOpenShell: () => openShell.mutate(card.id),
                              onOpenAgent: () => openAgent.mutate(card.id),
                            }
                          : null
                      }
                      onAddMeta={(args) => metaAdd.mutate({ cardId: card.id, ...args })}
                      onDeleteMeta={(id) => metaDelete.mutate(id)}
                      prActions={
                        card.type === "pr"
                          ? {
                              busy: prActionCardId === card.id,
                              onMerge: () => prMerge.mutate(card.id),
                              onSetDraft: (draft) => prSetDraft.mutate({ cardId: card.id, draft }),
                              onUpdateBranch: (rebase) => prUpdateBranch.mutate({ cardId: card.id, rebase }),
                            }
                          : null
                      }
                    />
                  ))}
                </div>
              </div>
            );
          })}
        </div>
      </div>

      {showTrackDialog && <TrackPrDialog onClose={() => setShowTrackDialog(false)} />}
    </section>
  );
}

function FilterList({
  title,
  entries,
  selected,
  onSelect,
  renderLabel,
}: {
  title: string;
  entries: [string, number][];
  selected: string | null;
  onSelect: (v: string | null) => void;
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
          {renderLabel ? renderLabel(key) : <span className="lbl">{key}</span>}
          <span className="count">{count}</span>
        </button>
      ))}
    </div>
  );
}
