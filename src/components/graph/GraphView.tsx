import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import { useInfiniteQuery, useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../../lib/api";
import { toast } from "../../lib/toast";
import { useSettingsStore } from "../../lib/settings";
import { timeAgo } from "../../lib/timeAgo";
import { useRepoWatch } from "../../lib/useRepoWatch";
import { useUIStore } from "../../store";
import type { RepoSyncState } from "../../lib/types";
import { KebabMenu } from "../KebabMenu";
import { layoutGraph } from "../../lib/graphLayout";
import type { ChangedFile, GraphCommit, GraphScope, RefInfo, Repository, WipKind } from "../../lib/types";
import { Icon } from "../icons";
import { CommitGraph, WIP_SELECTION, type CommitGraphHandle } from "./CommitGraph";
import { CommitPanel } from "./CommitPanel";
import { CommitReviewMenu } from "./CommitReviewMenu";
import { DiffPreview, type DiffPreviewTarget } from "./DiffPreview";
import { RepoSidebar } from "./RepoSidebar";
import { BranchActionsMenu, type MenuAnchor } from "./BranchActionsMenu";
import { CommitActionsMenu } from "./GitRefItems";
import { GitOpsProvider, useGitOps } from "./GitOps";
import "./GraphView.css";

export const GRAPH_PAGE_SIZE = 400;
export const MAX_JUMP_PAGES = 20;

/** Case-insensitive match on subject, body preview, author or sha prefix; every word must match. */
export function commitMatches(c: GraphCommit, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return false;
  const hay = `${c.subject}\n${c.body_preview}\n${c.author_name} ${c.author_email}`.toLowerCase();
  return words.every((w) => hay.includes(w) || c.sha.startsWith(w));
}
const OUT_OF_SCOPE_MESSAGE = "Switch to All to see this branch";

const SCOPE_OPTIONS: { value: GraphScope; label: string }[] = [
  { value: "local", label: "Local" },
  { value: "tracked", label: "Local + remotes" },
  { value: "all", label: "All" },
];

type WipEntry = { path: string; kind: WipKind };

const FETCHED_KEYS = ["graph", "refs", "worktrees", "branches", "repo-sync", "prs"];

export function fetchLabel(state: RepoSyncState | undefined): string {
  if (state?.last_fetch_error) return "fetch failed";
  if (state?.last_fetch_at) return `fetched ${timeAgo(state.last_fetch_at)}`;
  return "never fetched";
}

function basename(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

export function GraphView({ repo }: { repo: Repository }) {
  const queryClient = useQueryClient();
  const graphRef = useRef<CommitGraphHandle>(null);

  const [scope, setScope] = useState<GraphScope>("local");
  const [ctx, setCtx] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [previewIndex, setPreviewIndex] = useState<number | null>(null);
  const [pendingScroll, setPendingScroll] = useState<string | null>(null);
  const [jumpedRef, setJumpedRef] = useState<RefInfo | null>(null);
  const [pillMenu, setPillMenu] = useState<{ ref: RefInfo; anchor: MenuAnchor } | null>(null);
  const [commitMenu, setCommitMenu] = useState<{ sha: string; anchor: MenuAnchor } | null>(null);
  const [stashPreview, setStashPreview] = useState<number | null>(null);
  const [search, setSearch] = useState("");
  const [matchIndex, setMatchIndex] = useState(0);
  const searchRef = useRef<HTMLInputElement>(null);
  const [leftWidth, setLeftWidth] = useState(240);
  const [rightWidth, setRightWidth] = useState(340);
  const hasClone = repo.local_path != null;
  // Watched repos reload from git-changed events; the rest reload on focus.
  const watching = useRepoWatch(hasClone ? repo.id : null);

  const graphQuery = useInfiniteQuery({
    queryKey: ["graph", repo.id, scope, ctx],
    queryFn: ({ pageParam }) =>
      api.graphLog({
        repoId: repo.id,
        scope,
        worktreePath: ctx,
        skip: pageParam,
        limit: GRAPH_PAGE_SIZE,
      }),
    initialPageParam: 0,
    getNextPageParam: (last, all) =>
      last.length < GRAPH_PAGE_SIZE ? undefined : all.length * GRAPH_PAGE_SIZE,
    refetchOnWindowFocus: !watching,
  });

  const refsQuery = useQuery({
    queryKey: ["refs", repo.id, ctx],
    queryFn: () => api.listRefs(repo.id, ctx),
    refetchOnWindowFocus: !watching,
  });

  const wipQuery = useQuery({
    queryKey: ["wip", repo.id, ctx],
    queryFn: () => api.worktreeStatus(repo.id, ctx),
    refetchOnWindowFocus: true,
  });

  const worktreesQuery = useQuery({
    queryKey: ["worktrees", repo.id],
    queryFn: () => api.listWorktrees(repo.id),
    enabled: hasClone,
    refetchOnWindowFocus: !watching,
  });

  const syncQuery = useQuery({
    queryKey: ["repo-sync", repo.id],
    queryFn: () => api.repoSyncState(repo.id),
    enabled: hasClone,
  });
  const afterFetch = useCallback(() => {
    for (const key of FETCHED_KEYS) queryClient.invalidateQueries({ queryKey: [key, repo.id] });
    queryClient.invalidateQueries({ queryKey: ["repositories"] });
  }, [queryClient, repo.id]);
  const fetchRepo = useMutation({
    mutationFn: () => api.fetchRepo(repo.id),
    onSuccess: afterFetch,
    onError: (e) => {
      queryClient.invalidateQueries({ queryKey: ["repo-sync", repo.id] });
      toast.error(`Fetch failed:\n${String(e)}`);
    },
  });

  const autoFetchMinutes = useSettingsStore((s) => s.autoFetchMinutes);
  const visible = useUIStore((s) => s.activeTabId === "home" && s.homeRepoId === repo.id);
  useEffect(() => {
    if (!hasClone || !visible || autoFetchMinutes <= 0) return;
    const run = () => {
      api
        .fetchRepo(repo.id, autoFetchMinutes * 60)
        .then((r) => {
          if (r.fetched) afterFetch();
        })
        .catch(() => queryClient.invalidateQueries({ queryKey: ["repo-sync", repo.id] }));
    };
    run();
    window.addEventListener("focus", run);
    return () => window.removeEventListener("focus", run);
  }, [hasClone, visible, autoFetchMinutes, repo.id, afterFetch, queryClient]);

  // Keeps "fetched N min ago" current.
  const [, setTick] = useState(0);
  useEffect(() => {
    if (!syncQuery.data?.last_fetch_at) return;
    const id = setInterval(() => setTick((n) => n + 1), 30_000);
    return () => clearInterval(id);
  }, [syncQuery.data?.last_fetch_at]);

  const isCommitSelected = selected != null && selected !== WIP_SELECTION;
  const detailQuery = useQuery({
    queryKey: ["commit", repo.id, selected],
    queryFn: () => api.commitDetail(repo.id, selected!),
    enabled: isCommitSelected,
  });

  const pages = graphQuery.data?.pages;
  const commits = useMemo(() => (pages ?? []).flat(), [pages]);
  const layout = useMemo(() => layoutGraph(commits), [commits]);

  const matches = useMemo(
    () => (search.trim() ? commits.filter((c) => commitMatches(c, search)).map((c) => c.sha) : []),
    [commits, search],
  );
  const matchSet = useMemo(() => new Set(matches), [matches]);

  // Oldest loaded commit per author: the likeliest to be pushed, so GitHub can map it to a user.
  const authorShas = useMemo(() => {
    const m = new Map<string, string>();
    for (const c of commits) if (c.author_email) m.set(c.author_email.toLowerCase(), c.sha);
    return [...m];
  }, [commits]);
  const avatars = useQueries({
    queries: authorShas.map(([email, sha]) => ({
      queryKey: ["avatar", email],
      queryFn: () => api.commitAvatar(repo.id, email, sha),
      staleTime: Infinity,
      gcTime: Infinity,
      retry: false,
    })),
    combine: (results) => {
      const m = new Map<string, string>();
      results.forEach((r, i) => {
        if (r.data) m.set(authorShas[i][0], r.data);
      });
      return m;
    },
  });

  const refs = useMemo(() => refsQuery.data ?? [], [refsQuery.data]);

  const ghAuthQuery = useQuery({ queryKey: ["gh-auth"], queryFn: api.ghAuthStatus });
  const prsQuery = useQuery({
    queryKey: ["prs", repo.id],
    queryFn: () => api.listPrs(repo.id),
    enabled: ghAuthQuery.data === true && !!repo.remote_owner,
  });
  const worktrees = useMemo(() => worktreesQuery.data ?? [], [worktreesQuery.data]);
  const wip = wipQuery.data ?? null;
  const headSha = wip?.head_sha ?? refs.find((r) => r.is_head)?.sha ?? null;

  const wipEntries = useMemo<WipEntry[]>(() => {
    if (!wip) return [];
    return [
      ...wip.staged.map((f) => ({ path: f.path, kind: "staged" as const })),
      ...wip.unstaged.map((f) => ({ path: f.path, kind: "unstaged" as const })),
      ...wip.untracked.map((path) => ({ path, kind: "untracked" as const })),
    ];
  }, [wip]);
  const wipCount = useMemo(() => new Set(wipEntries.map((e) => e.path)).size, [wipEntries]);

  const detail = isCommitSelected && detailQuery.data?.sha === selected ? detailQuery.data : null;
  const commitFiles: ChangedFile[] = detail?.files ?? [];
  const previewList: string[] =
    selected === WIP_SELECTION ? wipEntries.map((e) => e.path) : commitFiles.map((f) => f.path);

  let previewTarget: DiffPreviewTarget | null = null;
  if (previewIndex != null && previewIndex < previewList.length) {
    if (selected === WIP_SELECTION) {
      const entry = wipEntries[previewIndex];
      previewTarget = {
        kind: "wip",
        repoId: repo.id,
        worktreePath: ctx,
        path: entry.path,
        wipKind: entry.kind,
      };
    } else if (detail) {
      previewTarget = { kind: "commit", repoId: repo.id, sha: detail.sha, file: commitFiles[previewIndex] };
    }
  }
  if (stashPreview != null) {
    previewTarget = { kind: "stash", repoId: repo.id, worktreePath: ctx, index: stashPreview };
  }
  const previewOpen = previewTarget != null;

  const closePreview = useCallback(() => {
    setPreviewIndex(null);
    setStashPreview(null);
  }, []);

  const select = useCallback((sha: string) => {
    setSelected(sha);
    setPreviewIndex(null);
    setStashPreview(null);
  }, []);

  const resetContext = useCallback((path: string | null) => {
    setCtx(path);
    setSelected(null);
    setPreviewIndex(null);
    setStashPreview(null);
    setPendingScroll(null);
    setJumpedRef(null);
  }, []);

  useEffect(() => {
    if (!pendingScroll) return;
    if (graphRef.current?.scrollToSha(pendingScroll)) setPendingScroll(null);
  }, [pendingScroll, commits, previewOpen]);

  const jumpSeq = useRef(0);
  const jumpToSha = useCallback(
    /** Resolves false when not found (toasting `notFoundMessage` if given). */
    async (sha: string, notFoundMessage: string | null): Promise<boolean> => {
      const seq = ++jumpSeq.current;
      const found = () => {
        select(sha);
        setPendingScroll(sha);
        return true;
      };
      if (commits.some((c) => c.sha === sha)) return found();

      let hasNext = graphQuery.hasNextPage;
      let pageCount = pages?.length ?? 0;
      while (hasNext && pageCount < MAX_JUMP_PAGES) {
        const res = await graphQuery.fetchNextPage();
        if (seq !== jumpSeq.current) return true;
        const loaded = res.data?.pages ?? [];
        if (loaded.length <= pageCount) break;
        pageCount = loaded.length;
        if (loaded.some((page) => page.some((c) => c.sha === sha))) return found();
        hasNext = res.hasNextPage;
      }
      if (notFoundMessage) toast.error(notFoundMessage);
      return false;
    },
    [commits, pages, graphQuery, select],
  );

  const goToMatch = useCallback(
    (index: number) => {
      if (matches.length === 0) return;
      const i = ((index % matches.length) + matches.length) % matches.length;
      setMatchIndex(i);
      select(matches[i]);
      setPendingScroll(matches[i]);
    },
    [matches, select],
  );

  // Cmd/Ctrl+F focuses the search box while the graph tab is showing.
  useEffect(() => {
    if (!visible) return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.altKey && e.key.toLowerCase() === "f") {
        e.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [visible]);

  // Jump to the first match once the query yields one (it may come from a later page).
  const awaitingFirst = useRef(false);
  useEffect(() => {
    awaitingFirst.current = true;
    setMatchIndex(0);
  }, [search]);
  useEffect(() => {
    if (!awaitingFirst.current || matches.length === 0) return;
    awaitingFirst.current = false;
    goToMatch(0);
  }, [matches, goToMatch]);

  // Matches may live beyond the loaded pages: keep loading (capped) until one shows up.
  useEffect(() => {
    if (!search.trim() || matches.length > 0) return;
    if (graphQuery.hasNextPage && !graphQuery.isFetchingNextPage && (pages?.length ?? 0) < MAX_JUMP_PAGES) {
      void graphQuery.fetchNextPage();
    }
  }, [search, matches.length, graphQuery, pages?.length]);

  const onJumpToRef = useCallback(
    (ref: RefInfo) => {
      setJumpedRef(ref);
      if (ref.kind === "remote" && scope === "local") {
        toast.error(OUT_OF_SCOPE_MESSAGE);
        return;
      }
      void jumpToSha(
        ref.sha,
        scope === "all" ? `${ref.name} is beyond the loaded history` : OUT_OF_SCOPE_MESSAGE,
      );
    },
    [jumpToSha, scope],
  );

  const onJumpToParent = useCallback(
    (sha: string) => {
      void jumpToSha(
        sha,
        scope === "all" ? `Commit ${sha.slice(0, 7)} is beyond the loaded history` : OUT_OF_SCOPE_MESSAGE,
      );
    },
    [jumpToSha, scope],
  );

  const graphFocus = useUIStore((s) => (s.graphFocus?.repoId === repo.id ? s.graphFocus : null));
  const clearGraphFocus = useUIStore((s) => s.clearGraphFocus);
  const focusGraph = useUIStore((s) => s.focusGraph);
  const graphLoaded = graphQuery.data != null && !graphQuery.isFetching;
  useEffect(() => {
    if (!graphFocus) return;
    if (graphFocus.worktree !== ctx) {
      resetContext(graphFocus.worktree);
      return;
    }
    if (graphFocus.sha) {
      // A PR head may sit only on a remote branch, or not be fetched yet.
      if (scope !== "all") {
        setScope("all");
        return;
      }
      if (!graphLoaded) return;
      const { sha, fetched } = graphFocus;
      const short = sha.slice(0, 7);
      clearGraphFocus();
      void jumpToSha(sha, fetched ? `Commit ${short} is not on any branch of this clone` : null).then((found) => {
        if (found || fetched) return;
        api
          .fetchRepo(repo.id, 0)
          .then(() => {
            afterFetch();
            focusGraph({ repoId: repo.id, ref: null, worktree: ctx, sha, fetched: true });
          })
          .catch((e) => toast.error(`Commit ${short} not found locally and fetch failed:\n${String(e)}`));
      });
      return;
    }
    if (!graphFocus.ref) {
      clearGraphFocus();
      return;
    }
    if (refsQuery.isError) {
      toast.error(`Could not load refs:\n${String(refsQuery.error)}`);
      clearGraphFocus();
      return;
    }
    if (refsQuery.data == null || refsQuery.isFetching) return;
    const name = graphFocus.ref;
    const ref =
      (["local", "tag", "remote"] as const)
        .map((kind) => refsQuery.data.find((r) => r.kind === kind && r.name === name))
        .find((r) => r != null) ?? null;
    if (!ref) {
      toast.error(`Ref ${name} not found`);
      clearGraphFocus();
      return;
    }
    if (ref.kind === "remote" && scope !== "all") {
      setScope("all");
      return;
    }
    if (!graphLoaded) return;
    onJumpToRef(ref);
    clearGraphFocus();
  }, [graphFocus, ctx, scope, refsQuery.data, refsQuery.isFetching, refsQuery.isError, refsQuery.error, graphLoaded, resetContext, onJumpToRef, clearGraphFocus, jumpToSha, focusGraph, afterFetch, repo.id]);

  const onEndReached = useCallback(() => {
    if (graphQuery.hasNextPage && !graphQuery.isFetchingNextPage) void graphQuery.fetchNextPage();
  }, [graphQuery]);

  const onRefContextMenu = useCallback((ref: RefInfo, e: ReactMouseEvent) => {
    setPillMenu({ ref, anchor: { x: e.clientX, y: e.clientY } });
  }, []);

  const onCommitContextMenu = useCallback((sha: string, e: ReactMouseEvent) => {
    setCommitMenu({ sha, anchor: { x: e.clientX, y: e.clientY } });
  }, []);

  const onSelectWorktree = useCallback(
    (path: string) => {
      const wt = worktrees.find((w) => w.path === path);
      resetContext(wt?.is_main ? null : path);
    },
    [worktrees, resetContext],
  );

  const onWorktreeRemoved = useCallback(
    (path: string) => {
      if (ctx === path) resetContext(null);
    },
    [ctx, resetContext],
  );

  const reload = () => {
    for (const key of ["graph", "refs", "wip", "worktrees"]) {
      queryClient.invalidateQueries({ queryKey: [key, repo.id] });
    }
  };
  const reloading =
    graphQuery.isFetching || refsQuery.isFetching || wipQuery.isFetching;

  const startResize = (side: "left" | "right") => (e: ReactMouseEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const startLeft = leftWidth;
    const startRight = rightWidth;
    const onMove = (ev: MouseEvent) => {
      const delta = ev.clientX - startX;
      if (side === "left") setLeftWidth(Math.max(160, Math.min(500, startLeft + delta)));
      else setRightWidth(Math.max(220, Math.min(640, startRight - delta)));
    };
    const onUp = () => {
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
    };
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  };

  const ctxWorktree = ctx ? worktrees.find((w) => w.path === ctx) : undefined;
  const ctxLabel = ctx ? ctxWorktree?.display_path ?? basename(ctx) : "main checkout";
  const ctxBranch = ctx ? ctxWorktree?.branch : refs.find((r) => r.kind === "local" && r.is_head)?.name;

  let centerBody;
  if (graphQuery.isPending) {
    centerBody = <p className="muted gv-msg">Loading history…</p>;
  } else if (graphQuery.isError) {
    centerBody = <p className="error gv-msg">Could not load history: {String(graphQuery.error)}</p>;
  } else if (commits.length === 0) {
    centerBody = <p className="muted gv-msg">No commits.</p>;
  } else {
    centerBody = (
      <CommitGraph
        ref={graphRef}
        commits={commits}
        rows={layout.rows}
        maxLanes={layout.maxLanes}
        avatars={avatars}
        refs={refs}
        worktrees={worktrees}
        headSha={headSha}
        wip={wipCount > 0 ? { count: wipCount } : null}
        selectedSha={selected}
        matchShas={matchSet}
        onSelect={select}
        arrowKeys={visible && !previewOpen}
        onEndReached={onEndReached}
        onRefContextMenu={onRefContextMenu}
        onCommitContextMenu={onCommitContextMenu}
      />
    );
  }

  let rightBody;
  if (selected === WIP_SELECTION && wip) {
    rightBody = (
      <CommitPanel
        wip={wip}
        activePath={previewTarget?.kind === "wip" ? previewTarget.path : null}
        onOpenWipFile={(path, kind) => {
          const idx = wipEntries.findIndex((e) => e.path === path && e.kind === kind);
          if (idx >= 0) setPreviewIndex(idx);
        }}
      />
    );
  } else if (detail) {
    const tipRefs = refs.filter((r) => r.sha === detail.sha && r.kind !== "tag" && !r.name.endsWith("/HEAD"));
    const tipBranches = new Set(
      tipRefs.map((r) => (r.kind === "remote" ? r.name.slice(r.name.indexOf("/") + 1) : r.name)),
    );
    const pullRequests = (prsQuery.data ?? []).filter((pr) => tipBranches.has(pr.headRefName));
    rightBody = (
      <CommitPanel
        detail={detail}
        activePath={previewTarget?.kind === "commit" ? previewTarget.file.path : null}
        onOpenFile={(file) => {
          const idx = commitFiles.findIndex((f) => f.path === file.path && f.old_path === file.old_path);
          if (idx >= 0) setPreviewIndex(idx);
        }}
        onJumpToSha={onJumpToParent}
        pullRequests={pullRequests}
        actions={
          repo.local_path || pullRequests.length > 0 ? (
            <CommitReviewMenu repo={repo} detail={detail} tipRefs={tipRefs} pullRequests={pullRequests} />
          ) : null
        }
        onOpenPr={(pr) => api.openUrl(pr.url).catch((e) => toast.error(`Could not open PR: ${String(e)}`))}
      />
    );
  } else if (isCommitSelected && detailQuery.isError) {
    rightBody = <p className="error gv-msg">Could not load commit: {String(detailQuery.error)}</p>;
  } else if (isCommitSelected) {
    rightBody = <p className="muted gv-msg">Loading commit…</p>;
  } else {
    rightBody = <p className="muted gv-msg">Select a commit to see its details.</p>;
  }

  const body = (
    <div className="gv-root">
      <div className="gv-toolbar">
        <label className="gv-scope">
          scope
          <select
            className="select"
            aria-label="Graph scope"
            value={scope}
            onChange={(e) => setScope(e.target.value as GraphScope)}
          >
            {SCOPE_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </label>
        <span className={`gv-ctx-chip${ctx ? " is-worktree" : ""}`} title={ctx ?? repo.local_path ?? undefined}>
          <Icon name="branch" size={12} />
          <span className="gv-ctx-label mono">{ctxLabel}</span>
          {ctxBranch && <span className="gv-ctx-branch mono">{ctxBranch}</span>}
          {ctx && (
            <button
              className="btn-icon gv-ctx-reset"
              title="Reset to main checkout"
              aria-label="Reset to main checkout"
              onClick={() => resetContext(null)}
            >
              <Icon name="x" size={11} />
            </button>
          )}
        </span>
        <span className="cr-spacer" />
        <div className="gv-search" role="search">
          <input
            ref={searchRef}
            className="input"
            type="search"
            placeholder="Search commits…"
            aria-label="Search commits"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                goToMatch(matchIndex + (e.shiftKey ? -1 : 1));
              } else if (e.key === "Escape") {
                setSearch("");
                e.currentTarget.blur();
              }
            }}
          />
          {search.trim() && (
            <>
              <span className="gv-search-count" aria-live="polite">
                {matches.length > 0
                  ? `${matchIndex + 1} / ${matches.length}${graphQuery.hasNextPage ? "+" : ""}`
                  : graphQuery.isFetchingNextPage || graphQuery.hasNextPage
                    ? "searching…"
                    : "no match"}
              </span>
              <button
                className="btn-icon"
                title="Previous match (Shift+Enter)"
                aria-label="Previous match"
                disabled={matches.length === 0}
                onClick={() => goToMatch(matchIndex - 1)}
              >
                ↑
              </button>
              <button
                className="btn-icon"
                title="Next match (Enter)"
                aria-label="Next match"
                disabled={matches.length === 0}
                onClick={() => goToMatch(matchIndex + 1)}
              >
                ↓
              </button>
            </>
          )}
        </div>
        {reloading && <span className="spinner" aria-label="Reloading" />}
        {hasClone && (
          <>
            <span
              className="cr-sub gv-fetched"
              title={syncQuery.data?.last_fetch_error ?? syncQuery.data?.last_fetch_at ?? undefined}
            >
              {fetchLabel(syncQuery.data)}
            </span>
            <button
              className="btn btn-sm"
              title="git fetch --prune origin"
              disabled={fetchRepo.isPending}
              onClick={() => fetchRepo.mutate()}
            >
              {fetchRepo.isPending ? <span className="spinner" /> : <Icon name="refresh" size={13} />}
              Fetch
            </button>
            <PullPushButtons branch={ctxBranch ?? null} />
          </>
        )}
        <KebabMenu title="Graph actions">
          {(close) => (
            <button
              role="menuitem"
              title="Re-read local git state (normally automatic)"
              onClick={() => {
                close();
                reload();
              }}
            >
              Reload
            </button>
          )}
        </KebabMenu>
      </div>

      <div className="gv-panes">
        <div className="gv-pane gv-left" style={{ width: leftWidth }}>
          <RepoSidebar
            repo={repo}
            refs={refs}
            worktrees={worktrees}
            contextPath={ctx}
            onJumpToRef={onJumpToRef}
            onSelectWorktree={onSelectWorktree}
            onWorktreeRemoved={onWorktreeRemoved}
            selectedRefName={jumpedRef && jumpedRef.sha === selected ? jumpedRef.name : null}
            onPreviewStash={(index) => {
              setSelected(null);
              setPreviewIndex(null);
              setStashPreview(index);
            }}
          />
        </div>
        <div className="resize-handle" onMouseDown={startResize("left")} />
        <div className="gv-pane gv-center">
          <div className="gv-graph" style={previewOpen ? { display: "none" } : undefined}>
            {centerBody}
          </div>
          {previewTarget && (
            <DiffPreview
              target={previewTarget}
              files={previewTarget.kind === "stash" ? [`stash@{${previewTarget.index}}`] : previewList}
              index={previewTarget.kind === "stash" ? 0 : previewIndex!}
              onNavigate={setPreviewIndex}
              onClose={closePreview}
              keyboard={visible}
            />
          )}
        </div>
        <div className="resize-handle" onMouseDown={startResize("right")} />
        <div className="gv-pane gv-right" style={{ width: rightWidth }}>
          {rightBody}
        </div>
      </div>

      {pillMenu && (
        <BranchActionsMenu
          repo={repo}
          branch={pillMenu.ref.name}
          refInfo={pillMenu.ref}
          anchor={pillMenu.anchor}
          onClose={() => setPillMenu(null)}
        />
      )}
      {commitMenu && (
        <CommitActionsMenu
          sha={commitMenu.sha}
          refs={refs.filter((r) => r.sha === commitMenu.sha)}
          anchor={commitMenu.anchor}
          onClose={() => setCommitMenu(null)}
        />
      )}
    </div>
  );

  if (!hasClone) return body;
  return (
    <GitOpsProvider
      repo={repo}
      ctx={ctx}
      refs={refs}
      worktrees={worktrees}
      onSwitchContext={onSelectWorktree}
    >
      {body}
    </GitOpsProvider>
  );
}

function PullPushButtons({ branch }: { branch: string | null }) {
  const ops = useGitOps();
  if (!ops) return null;
  const title = branch ? undefined : "HEAD is detached";
  return (
    <>
      <button
        className="btn btn-sm"
        title={title ?? `git pull --ff-only (${branch})`}
        disabled={!branch || ops.busy}
        onClick={ops.pull}
      >
        Pull
      </button>
      <button
        className="btn btn-sm"
        title={title ?? `git push (${branch})`}
        disabled={!branch || ops.busy}
        onClick={ops.push}
      >
        Push
      </button>
    </>
  );
}
