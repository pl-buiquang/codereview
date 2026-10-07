import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import { useInfiniteQuery, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../../lib/api";
import { toast } from "../../lib/toast";
import { layoutGraph } from "../../lib/graphLayout";
import type { ChangedFile, GraphScope, RefInfo, Repository, WipKind } from "../../lib/types";
import { Icon } from "../icons";
import { CommitGraph, WIP_SELECTION, type CommitGraphHandle } from "./CommitGraph";
import { CommitPanel } from "./CommitPanel";
import { CommitReviewMenu } from "./CommitReviewMenu";
import { DiffPreview, type DiffPreviewTarget } from "./DiffPreview";
import { RepoSidebar } from "./RepoSidebar";
import { BranchActionsMenu, type MenuAnchor } from "./BranchActionsMenu";
import "./GraphView.css";

export const GRAPH_PAGE_SIZE = 400;
export const MAX_JUMP_PAGES = 20;
const OUT_OF_SCOPE_MESSAGE = "Switch to All to see this branch";

const SCOPE_OPTIONS: { value: GraphScope; label: string }[] = [
  { value: "local", label: "Local" },
  { value: "all", label: "All" },
];

type WipEntry = { path: string; kind: WipKind };

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
  const [pillMenu, setPillMenu] = useState<{ branch: string; anchor: MenuAnchor } | null>(null);
  const [leftWidth, setLeftWidth] = useState(240);
  const [rightWidth, setRightWidth] = useState(340);

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
  });

  const refsQuery = useQuery({
    queryKey: ["refs", repo.id, ctx],
    queryFn: () => api.listRefs(repo.id, ctx),
  });

  const wipQuery = useQuery({
    queryKey: ["wip", repo.id, ctx],
    queryFn: () => api.worktreeStatus(repo.id, ctx),
    refetchOnWindowFocus: true,
  });

  const worktreesQuery = useQuery({
    queryKey: ["worktrees", repo.id],
    queryFn: () => api.listWorktrees(repo.id),
    enabled: repo.local_path != null,
  });

  const isCommitSelected = selected != null && selected !== WIP_SELECTION;
  const detailQuery = useQuery({
    queryKey: ["commit", repo.id, selected],
    queryFn: () => api.commitDetail(repo.id, selected!),
    enabled: isCommitSelected,
  });

  const pages = graphQuery.data?.pages;
  const commits = useMemo(() => (pages ?? []).flat(), [pages]);
  const layout = useMemo(() => layoutGraph(commits), [commits]);

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
  const previewOpen = previewTarget != null;

  const closePreview = useCallback(() => setPreviewIndex(null), []);

  const select = useCallback((sha: string) => {
    setSelected(sha);
    setPreviewIndex(null);
  }, []);

  const resetContext = useCallback((path: string | null) => {
    setCtx(path);
    setSelected(null);
    setPreviewIndex(null);
    setPendingScroll(null);
    setJumpedRef(null);
  }, []);

  useEffect(() => {
    if (!pendingScroll) return;
    if (graphRef.current?.scrollToSha(pendingScroll)) setPendingScroll(null);
  }, [pendingScroll, commits, previewOpen]);

  const jumpSeq = useRef(0);
  const jumpToSha = useCallback(
    async (sha: string, notFoundMessage: string) => {
      const seq = ++jumpSeq.current;
      const found = () => {
        select(sha);
        setPendingScroll(sha);
      };
      if (commits.some((c) => c.sha === sha)) return found();

      let hasNext = graphQuery.hasNextPage;
      let pageCount = pages?.length ?? 0;
      while (hasNext && pageCount < MAX_JUMP_PAGES) {
        const res = await graphQuery.fetchNextPage();
        if (seq !== jumpSeq.current) return;
        const loaded = res.data?.pages ?? [];
        if (loaded.length <= pageCount) break;
        pageCount = loaded.length;
        if (loaded.some((page) => page.some((c) => c.sha === sha))) return found();
        hasNext = res.hasNextPage;
      }
      toast.error(notFoundMessage);
    },
    [commits, pages, graphQuery, select],
  );

  const onJumpToRef = useCallback(
    (ref: RefInfo) => {
      setJumpedRef(ref);
      if (ref.kind === "remote" && scope !== "all") {
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

  const onEndReached = useCallback(() => {
    if (graphQuery.hasNextPage && !graphQuery.isFetchingNextPage) void graphQuery.fetchNextPage();
  }, [graphQuery]);

  const onRefContextMenu = useCallback((ref: RefInfo, e: ReactMouseEvent) => {
    if (ref.kind === "tag") return;
    setPillMenu({ branch: ref.name, anchor: { x: e.clientX, y: e.clientY } });
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

  const refresh = () => {
    for (const key of ["graph", "refs", "wip", "worktrees"]) {
      queryClient.invalidateQueries({ queryKey: [key, repo.id] });
    }
  };
  const refreshing =
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
        onSelect={select}
        onEndReached={onEndReached}
        onRefContextMenu={onRefContextMenu}
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

  return (
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
        <button
          className="btn btn-sm"
          title="Refresh graph"
          aria-label="Refresh graph"
          disabled={refreshing}
          onClick={refresh}
        >
          {refreshing ? <span className="spinner" /> : <Icon name="refresh" size={13} />}
        </button>
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
              files={previewList}
              index={previewIndex!}
              onNavigate={setPreviewIndex}
              onClose={closePreview}
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
          branch={pillMenu.branch}
          anchor={pillMenu.anchor}
          onClose={() => setPillMenu(null)}
        />
      )}
    </div>
  );
}
