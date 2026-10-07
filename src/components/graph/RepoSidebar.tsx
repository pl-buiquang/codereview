import { useMemo, useState, type ReactNode } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "../../lib/api";
import { toast } from "../../lib/toast";
import { confirmDialog } from "../../lib/confirm";
import { Icon } from "../icons";
import type {
  PrSummary,
  RefInfo,
  Repository,
  WorktreeInfo,
  WorktreeSource,
} from "../../lib/types";
import { ancestorPaths, buildRefTree, type RefSort, type RefTreeNode } from "../../lib/refTree";
import { BranchActionsMenu, type MenuAnchor } from "./BranchActionsMenu";
import "./RepoSidebar.css";

const SOURCE_LABELS: Record<WorktreeSource, string> = {
  claude: "Claude",
  codex: "Codex",
  codereview: "CodeReview",
  manual: "manual",
};

type SectionId = "local" | "remote" | "worktrees" | "tags";

export interface RepoSidebarProps {
  repo: Repository;
  refs: RefInfo[];
  worktrees: WorktreeInfo[];
  /** Active graph context checkout; `null` means the repo's main checkout. */
  contextPath: string | null;
  onJumpToRef: (ref: RefInfo) => void;
  onSelectWorktree: (path: string) => void;
  /** Called after a worktree is removed (e.g. to reset the context if it was active). */
  onWorktreeRemoved?: (path: string) => void;
  /** Opens a newly created review; defaults to the UI store's `openReview`. */
  openReview?: (reviewId: number) => void;
  /** Highlights the ref row matching this name (e.g. the selected graph ref). */
  selectedRefName?: string | null;
}

function matches(text: string, filter: string): boolean {
  return filter === "" || text.toLowerCase().includes(filter.toLowerCase());
}

export function RepoSidebar({
  repo,
  refs,
  worktrees,
  contextPath,
  onJumpToRef,
  onSelectWorktree,
  onWorktreeRemoved,
  openReview,
  selectedRefName,
}: RepoSidebarProps) {
  const queryClient = useQueryClient();
  const [collapsed, setCollapsed] = useState<Record<SectionId, boolean>>({
    local: false,
    remote: true,
    worktrees: true,
    tags: true,
  });
  const [filters, setFilters] = useState<Record<SectionId, string>>({
    local: "",
    remote: "",
    worktrees: "",
    tags: "",
  });
  const [menu, setMenu] = useState<{ branch: string; anchor: MenuAnchor } | null>(null);
  const [sort, setSort] = useState<RefSort>("updated");
  const [toggledFolders, setToggledFolders] = useState<ReadonlySet<string>>(() => new Set());

  const { local, remote, tags } = useMemo(() => {
    const local: RefInfo[] = [];
    const remote: RefInfo[] = [];
    const tags: RefInfo[] = [];
    for (const r of refs) {
      if (r.kind === "local") local.push(r);
      else if (r.kind === "remote") {
        // `refs/remotes/<remote>/HEAD` is a symbolic alias, not a branch.
        if (!r.name.endsWith("/HEAD")) remote.push(r);
      } else tags.push(r);
    }
    return { local, remote, tags };
  }, [refs]);

  const invalidateWorktrees = () =>
    queryClient.invalidateQueries({ queryKey: ["worktrees", repo.id] });

  const removeWorktree = useMutation({
    mutationFn: (path: string) => api.removeWorktree(repo.id, path),
    onSuccess: (_data, path) => {
      invalidateWorktrees();
      onWorktreeRemoved?.(path);
    },
    onError: (e) => toast.error(`Remove failed: ${String(e)}`),
  });

  const pruneWorktrees = useMutation({
    mutationFn: () => api.pruneWorktrees(repo.id),
    onSuccess: () => {
      invalidateWorktrees();
      toast.success("Stale worktrees pruned.");
    },
    onError: (e) => toast.error(`Prune failed: ${String(e)}`),
  });

  const pruneGone = useMutation({
    mutationFn: async () => {
      const gone = await api.listGoneBranches(repo.id);
      if (gone.length === 0) return null;
      const ok = await confirmDialog({
        title: "Delete gone branches",
        message: `Force-delete ${gone.length} local branch${gone.length === 1 ? "" : "es"} whose upstream is gone?\n\n${gone.join("\n")}`,
        confirmLabel: "Delete",
        danger: true,
      });
      return ok ? api.deleteGoneBranches(repo.id, gone) : undefined;
    },
    onSuccess: (outcome) => {
      if (outcome === null) {
        toast.success("No gone branches.");
        return;
      }
      if (!outcome) return;
      queryClient.invalidateQueries({ queryKey: ["refs", repo.id] });
      queryClient.invalidateQueries({ queryKey: ["graph", repo.id] });
      if (outcome.deleted.length) toast.success(`Deleted ${outcome.deleted.length} gone branch(es).`);
      for (const f of outcome.failed) toast.error(`Could not delete ${f.name}: ${f.error}`);
    },
    onError: (e) => toast.error(`Prune failed: ${String(e)}`),
  });

  const hasPrunable = worktrees.some((w) => w.is_prunable);
  // Use cached PR data if available (populated when GitHub PRs tab has been visited).
  const cachedPrs = queryClient.getQueryData<PrSummary[]>(["prs", repo.id]) ?? [];

  const toggle = (id: SectionId) => setCollapsed((c) => ({ ...c, [id]: !c[id] }));
  const setFilter = (id: SectionId, value: string) => setFilters((f) => ({ ...f, [id]: value }));

  const openMenu = (branch: string, anchor: MenuAnchor) => setMenu({ branch, anchor });

  const activeNames = useMemo(
    () => new Set([...refs.filter((r) => r.is_head).map((r) => r.name), ...(selectedRefName ? [selectedRefName] : [])]),
    [refs, selectedRefName],
  );
  const openByDefault = (id: SectionId, path: string, depth: number) =>
    (id === "remote" && depth === 0) ||
    [...activeNames].some((n) => ancestorPaths(n).includes(path));
  const toggleFolder = (key: string) =>
    setToggledFolders((prev) => {
      const next = new Set(prev);
      if (!next.delete(key)) next.add(key);
      return next;
    });

  const renderTree = (nodes: RefTreeNode[], id: SectionId, withActions: boolean, depth: number): ReactNode =>
    nodes.map((node) => {
      if (node.type === "leaf") {
        const r = node.ref;
        return (
          <RefRow
            key={`${r.kind}:${r.name}`}
            refInfo={r}
            label={node.label}
            depth={depth}
            selected={selectedRefName === r.name}
            onJump={() => onJumpToRef(r)}
            onOpenMenu={withActions ? (anchor) => openMenu(r.name, anchor) : undefined}
          />
        );
      }
      const key = `${id}:${node.path}`;
      const open = filters[id] !== "" || openByDefault(id, node.path, depth) !== toggledFolders.has(key);
      return (
        <div key={key} className="rs-folder">
          <button
            className="rs-folder-row"
            style={{ paddingLeft: 6 + depth * 12 }}
            aria-expanded={open}
            title={node.path}
            onClick={() => toggleFolder(key)}
          >
            <span className={`rs-chev${open ? "" : " collapsed"}`}>
              <Icon name="chev" size={10} />
            </span>
            <span className="rs-row-name">{node.label}</span>
            <span className="rs-folder-count">{node.count}</span>
          </button>
          {open && renderTree(node.children, id, withActions, depth + 1)}
        </div>
      );
    });

  const renderRefs = (list: RefInfo[], id: SectionId, withActions: boolean) =>
    renderTree(buildRefTree(list.filter((r) => matches(r.name, filters[id])), sort), id, withActions, 0);

  const visibleWorktrees = worktrees.filter(
    (w) => matches(w.display_path, filters.worktrees) || matches(w.branch ?? "", filters.worktrees),
  );

  return (
    <aside className="rs-sidebar" aria-label="Repository refs">
      <div className="rs-toolbar">
        <button
          className="btn btn-sm"
          onClick={() => setSort((v) => (v === "updated" ? "name" : "updated"))}
          title="Toggle branch/tag sort order"
          aria-label={`Sort by ${sort === "updated" ? "last updated" : "name"}`}
        >
          Sort: {sort === "updated" ? "Last updated" : "Name"}
        </button>
      </div>
      <Section
        id="local"
        title="Local"
        total={local.length}
        shown={local.filter((r) => matches(r.name, filters.local)).length}
        collapsed={collapsed.local}
        filter={filters.local}
        onToggle={toggle}
        onFilter={setFilter}
        actions={
          <button
            className="btn btn-sm"
            disabled={pruneGone.isPending}
            onClick={() => pruneGone.mutate()}
            title="git fetch --prune, then delete local branches whose upstream is gone"
          >
            {pruneGone.isPending ? <span className="spinner" /> : "Prune gone"}
          </button>
        }
      >
        {renderRefs(local, "local", true)}
      </Section>

      <Section
        id="remote"
        title="Remote"
        total={remote.length}
        shown={remote.filter((r) => matches(r.name, filters.remote)).length}
        collapsed={collapsed.remote}
        filter={filters.remote}
        onToggle={toggle}
        onFilter={setFilter}
      >
        {renderRefs(remote, "remote", true)}
      </Section>

      <Section
        id="worktrees"
        title="Worktrees"
        total={worktrees.length}
        shown={visibleWorktrees.length}
        collapsed={collapsed.worktrees}
        filter={filters.worktrees}
        onToggle={toggle}
        onFilter={setFilter}
        actions={
          hasPrunable ? (
            <button
              className="btn btn-sm"
              disabled={pruneWorktrees.isPending}
              onClick={() => pruneWorktrees.mutate()}
              title="Prune stale worktrees"
            >
              Prune stale
            </button>
          ) : null
        }
      >
        {visibleWorktrees.map((wt) => {
          const linkedPr = wt.branch
            ? cachedPrs.find((pr) => pr.headRefName === wt.branch)
            : undefined;
          const active = contextPath === wt.path || (contextPath == null && wt.is_main);
          return (
            <WorktreeRow
              key={wt.path}
              wt={wt}
              active={active}
              linkedPr={linkedPr ?? null}
              onSelect={() => onSelectWorktree(wt.path)}
              onRemove={async () => {
                if (
                  await confirmDialog({
                    title: "Remove worktree",
                    message: `Remove worktree at ${wt.display_path}?\nThis deletes the working directory.`,
                    confirmLabel: "Remove",
                    danger: true,
                  })
                )
                  removeWorktree.mutate(wt.path);
              }}
            />
          );
        })}
      </Section>

      <Section
        id="tags"
        title="Tags"
        total={tags.length}
        shown={tags.filter((r) => matches(r.name, filters.tags)).length}
        collapsed={collapsed.tags}
        filter={filters.tags}
        onToggle={toggle}
        onFilter={setFilter}
      >
        {renderRefs(tags, "tags", false)}
      </Section>

      {menu && (
        <BranchActionsMenu
          repo={repo}
          branch={menu.branch}
          anchor={menu.anchor}
          onClose={() => setMenu(null)}
          onReviewCreated={openReview}
        />
      )}
    </aside>
  );
}

function Section({
  id,
  title,
  total,
  shown,
  collapsed,
  filter,
  onToggle,
  onFilter,
  actions,
  children,
}: {
  id: SectionId;
  title: string;
  total: number;
  shown: number;
  collapsed: boolean;
  filter: string;
  onToggle: (id: SectionId) => void;
  onFilter: (id: SectionId, value: string) => void;
  actions?: ReactNode;
  children: ReactNode;
}) {
  const bodyId = `rs-section-${id}`;
  return (
    <section className="rs-section" data-section={id}>
      <div className="rs-section-head">
        <button
          className="rs-section-toggle"
          aria-expanded={!collapsed}
          aria-controls={bodyId}
          onClick={() => onToggle(id)}
        >
          <span className={`rs-chev${collapsed ? " collapsed" : ""}`}>
            <Icon name="chev" size={11} />
          </span>
          <span className="rs-section-title">{title}</span>
          <span className="rs-count" data-testid={`count-${id}`}>
            {filter ? `${shown}/${total}` : total}
          </span>
        </button>
        {actions}
      </div>
      {!collapsed && (
        <div className="rs-section-body" id={bodyId}>
          <input
            className="input rs-filter"
            type="search"
            placeholder="Filter…"
            aria-label={`Filter ${title.toLowerCase()}`}
            value={filter}
            onChange={(e) => onFilter(id, e.target.value)}
          />
          <div className="rs-list">{children}</div>
          {shown === 0 && <p className="rs-empty faint">{total === 0 ? "None" : "No matches"}</p>}
        </div>
      )}
    </section>
  );
}

function RefRow({
  refInfo,
  label,
  depth,
  selected,
  onJump,
  onOpenMenu,
}: {
  refInfo: RefInfo;
  label: string;
  depth: number;
  selected: boolean;
  onJump: () => void;
  onOpenMenu?: (anchor: MenuAnchor) => void;
}) {
  return (
    <div
      className={`rs-row${selected ? " selected" : ""}`}
      onContextMenu={
        onOpenMenu
          ? (e) => {
              e.preventDefault();
              onOpenMenu({ x: e.clientX, y: e.clientY });
            }
          : undefined
      }
    >
      <button
        className="rs-row-main"
        style={{ paddingLeft: 6 + depth * 12 }}
        onClick={onJump}
        title={`Jump to ${refInfo.name}`}
        aria-label={`Jump to ${refInfo.name}`}
      >
        <Icon name={refInfo.kind === "tag" ? "link" : "branch"} size={12} />
        <span className="rs-row-name mono">{label}</span>
        {refInfo.is_head && <span className="rs-head-badge">HEAD</span>}
      </button>
      {onOpenMenu && (
        <button
          className="btn-icon rs-row-more"
          aria-label={`Actions for ${refInfo.name}`}
          title="Branch actions"
          onClick={(e) => {
            const rect = e.currentTarget.getBoundingClientRect();
            onOpenMenu({ x: rect.left, y: rect.bottom + 2 });
          }}
        >
          ⋯
        </button>
      )}
    </div>
  );
}

function WorktreeRow({
  wt,
  active,
  linkedPr,
  onSelect,
  onRemove,
}: {
  wt: WorktreeInfo;
  active: boolean;
  linkedPr: PrSummary | null;
  onSelect: () => void;
  onRemove: () => void;
}) {
  return (
    <div
      className={`rs-wt-row${active ? " active" : ""}`}
      role="button"
      tabIndex={0}
      aria-pressed={active}
      title={wt.path}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.target !== e.currentTarget) return;
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
    >
      <div className="rs-wt-main">
        <span className="rs-wt-path mono">{wt.display_path}</span>
        <div className="rs-wt-meta">
          {wt.branch ? (
            <span className="mono">{wt.branch}</span>
          ) : (
            <span className="mono faint">detached {wt.head_sha.slice(0, 8)}</span>
          )}
          {linkedPr && <span className="badge badge-pr rs-wt-badge">PR #{linkedPr.number}</span>}
          {wt.is_main && <span className="rs-wt-badge rs-wt-badge-main">main</span>}
          {wt.is_prunable && <span className="rs-wt-badge rs-wt-badge-stale">stale</span>}
          <span className={`rs-wt-source rs-wt-source-${wt.source}`}>
            {SOURCE_LABELS[wt.source]}
          </span>
        </div>
      </div>
      <button
        className="btn-icon"
        title="Open in VSCode"
        aria-label={`Open ${wt.display_path} in VSCode`}
        onClick={(e) => {
          e.stopPropagation();
          api.openInVscode(wt.path).catch((err) =>
            toast.error(`Could not open VSCode: ${String(err)}`),
          );
        }}
      >
        <Icon name="ext" size={11} />
      </button>
      {!wt.is_main && (
        <button
          className="btn-icon"
          title="Remove worktree"
          aria-label={`Remove worktree ${wt.display_path}`}
          onClick={(e) => {
            e.stopPropagation();
            onRemove();
          }}
        >
          <Icon name="x" size={12} />
        </button>
      )}
    </div>
  );
}
