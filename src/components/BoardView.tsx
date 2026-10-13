import { useEffect, useMemo, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { confirmDialog } from "../lib/confirm";
import { BOARD_POLL_OPTIONS, parseRepoBasePaths, useSettingsStore } from "../lib/settings";
import { useUIStore } from "../store";
import type { BoardCard as BoardCardT, BoardLane, BoardMetaKind } from "../lib/types";
import { BoardCard, prStatus, prStatusLabel } from "./BoardCard";
import { RepoName } from "./RepoName";
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
  const setBoardPollMs = useSettingsStore((s) => s.setBoardPollMs);
  const [repoFilter, setRepoFilter] = useState<string | null>(null);
  const [statusFilter, setStatusFilter] = useState<string | null>(null);
  const [addingLane, setAddingLane] = useState<BoardLane | null>(null);
  const [addValue, setAddValue] = useState("");

  const boardQuery = useQuery({ queryKey: ["board"], queryFn: api.boardList });
  const tagsQuery = useQuery({ queryKey: ["board-meta-tags"], queryFn: api.boardMetaTags });
  const repositoriesQuery = useQuery({ queryKey: ["repositories"], queryFn: api.listRepositories });

  const openCardRepo = (repo: string) => {
    const match = repositoriesQuery.data?.find(
      (r) => r.remote_owner && r.remote_name && `${r.remote_owner}/${r.remote_name}`.toLowerCase() === repo.toLowerCase(),
    );
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

  const { repoEntries, statusEntries } = useMemo(() => {
    const repo = new Map<string, number>();
    const status = new Map<string, number>();
    for (const c of allCards) {
      repo.set(c.repo, (repo.get(c.repo) ?? 0) + 1);
      const s = prStatus(c);
      if (s) status.set(s, (status.get(s) ?? 0) + 1);
    }
    const byCount = (a: [string, number], b: [string, number]) => b[1] - a[1] || a[0].localeCompare(b[0]);
    const byStatusOrder = (a: [string, number], b: [string, number]) =>
      (STATUS_ORDER[a[0]] ?? 99) - (STATUS_ORDER[b[0]] ?? 99);
    return {
      repoEntries: [...repo.entries()].sort(byCount),
      statusEntries: [...status.entries()].sort(byStatusOrder),
    };
  }, [allCards]);

  const effRepo = repoFilter && allCards.some((c) => c.repo === repoFilter) ? repoFilter : null;
  const effStatus = statusFilter && allCards.some((c) => prStatus(c) === statusFilter) ? statusFilter : null;

  const filtered = allCards.filter((c) => {
    if (effRepo && c.repo !== effRepo) return false;
    if (effStatus && c.type === "pr" && prStatus(c) !== effStatus) return false;
    return true;
  });

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
        </div>
      </header>

      <div className="inbox-layout">
        <aside className="cr-rail">
          <FilterList
            title="Status"
            entries={statusEntries}
            selected={effStatus}
            onSelect={setStatusFilter}
            renderLabel={(k) => <span className="lbl">{prStatusLabel(k)}</span>}
          />
          <FilterList
            title="Repositories"
            entries={repoEntries}
            selected={effRepo}
            onSelect={setRepoFilter}
            renderLabel={(k) => <RepoName className="lbl mono" name={k} />}
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
                      onAddMeta={(args) => metaAdd.mutate({ cardId: card.id, ...args })}
                      onDeleteMeta={(id) => metaDelete.mutate(id)}
                    />
                  ))}
                </div>
              </div>
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
