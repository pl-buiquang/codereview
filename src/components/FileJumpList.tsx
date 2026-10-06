import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type MutableRefObject,
  type ReactNode,
  type RefObject,
} from "react";
import { useQuery } from "@tanstack/react-query";
import { parseDiff } from "react-diff-view";
import { api } from "../lib/api";
import { countChanges, fileDisplayPath } from "../lib/diff";
import { isDraftReply } from "../lib/threads";
import { summaryLine } from "../lib/text";
import { timeAgo } from "../lib/timeAgo";
import type { JumpListHandle } from "../lib/keyboard";
import type { PrThread } from "../lib/types";
import { Icon } from "./icons";

interface Row {
  index: number;
  path: string;
  add: number;
  del: number;
  count: number;
  viewed: boolean;
}

type TreeNode =
  | { kind: "file"; name: string; row: Row }
  | { kind: "dir"; name: string; path: string; children: TreeNode[] };

/** Group flat file rows into a directory tree, GitHub-style: directory chains
 *  with a single child are collapsed into one node (`src/components`). Children
 *  keep their diff insertion order — never re-sorted — so the tree reads in the
 *  exact same order as the diff pane (git emits paths in byte-wise/C order, which
 *  is contiguous per directory, so first-seen order reproduces it). A locale sort
 *  would diverge on uppercase and dotted names. */
function buildTree(rows: Row[]): TreeNode[] {
  const root: Extract<TreeNode, { kind: "dir" }> = {
    kind: "dir",
    name: "",
    path: "",
    children: [],
  };
  for (const row of rows) {
    const parts = row.path.split("/");
    const fileName = parts.pop() ?? row.path;
    let dir = root;
    let prefix = "";
    for (const part of parts) {
      prefix = prefix ? `${prefix}/${part}` : part;
      let next = dir.children.find(
        (c): c is Extract<TreeNode, { kind: "dir" }> =>
          c.kind === "dir" && c.name === part,
      );
      if (!next) {
        next = { kind: "dir", name: part, path: prefix, children: [] };
        dir.children.push(next);
      }
      dir = next;
    }
    dir.children.push({ kind: "file", name: fileName, row });
  }
  root.children = root.children.map(compress);
  return root.children;
}

function compress(node: TreeNode): TreeNode {
  if (node.kind === "file") return node;
  node.children = node.children.map(compress);
  while (node.children.length === 1 && node.children[0].kind === "dir") {
    const only = node.children[0];
    node.name = node.name ? `${node.name}/${only.name}` : only.name;
    node.path = only.path;
    node.children = only.children;
  }
  return node;
}

function Chevron({ open }: { open: boolean }) {
  return (
    <svg
      className={`tree-chevron${open ? " open" : ""}`}
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="m9 6 6 6-6 6" />
    </svg>
  );
}

type ResolutionFilter = "all" | "unresolved" | "resolved";
type SourceFilter = "all" | "draft" | "published" | "github";
type SortMode = "file" | "time";

interface ReplyEntry {
  key: string;
  body: string;
  createdAt: string;
  source: "draft" | "published" | "github";
}

interface DisplayItem {
  key: string;
  filePath: string;
  line: number | null;
  startLine: number | null;
  isFileLevel: boolean;
  body: string;
  isResolved: boolean;
  createdAt: string;
  source: "draft" | "published" | "github";
  scrollId: string;
  scrollAttr: "comment-id" | "thread-id";
  replies: ReplyEntry[];
}

export function FileJumpList({
  reviewId,
  scrollRootRef,
  controlRef,
  paneCollapsed,
  onToggle,
  threads = [],
}: {
  reviewId: number;
  scrollRootRef: RefObject<HTMLElement | null>;
  controlRef?: MutableRefObject<JumpListHandle | null>;
  paneCollapsed?: boolean;
  onToggle?: () => void;
  threads?: PrThread[];
}) {
  const detailQuery = useQuery({
    queryKey: ["review", reviewId],
    queryFn: () => api.getReview(reviewId),
    enabled: reviewId != null,
  });
  const detail = detailQuery.data;

  const diffQuery = useQuery({
    queryKey: ["review-diff", reviewId, detail?.target.id, detail?.target.head_sha],
    enabled: detail != null,
    queryFn: () => api.reviewDiff(reviewId),
  });
  const diff = diffQuery.data;

  const files = useMemo(() => (diff ? parseDiff(diff) : []), [diff]);

  const countByPath = useMemo(() => {
    const map = new Map<string, number>();
    for (const c of detail?.comments ?? []) {
      map.set(c.file_path, (map.get(c.file_path) ?? 0) + 1);
    }
    return map;
  }, [detail?.comments]);

  const viewedSet = useMemo(
    () => new Set(detail?.viewed_files ?? []),
    [detail?.viewed_files],
  );

  const rows = useMemo(
    () =>
      files.map((file, index) => {
        const path = fileDisplayPath(file);
        const { add, del } = countChanges(file);
        return {
          index,
          path,
          add,
          del,
          count: countByPath.get(path) ?? 0,
          viewed: viewedSet.has(path),
        };
      }),
    [files, countByPath, viewedSet],
  );

  const [activeIndex, setActiveIndex] = useState(0);
  // A click selects a file outright and "locks" auto-selection: the scrollspy is
  // muted while the click-triggered smooth scroll plays out, then released once
  // scrolling goes quiet — without re-picking — so the chosen file stays selected
  // until the user actually scrolls again. (Otherwise the smooth scroll, or the
  // bottom-of-view fallback, would immediately steal the selection back — which
  // is why a trailing file that already fits on screen couldn't stay selected.)
  const lockedRef = useRef(false);
  const releaseTimerRef = useRef(0);

  // Track the active file from scroll position, scoping every `#file-N` lookup
  // to this review's own scroll panel. Other mounted review tabs render the same
  // `file-N` ids, so a document-wide getElementById would resolve to whichever
  // tab is first in the DOM (often a display:none one) — which is why the jump
  // list reacted on one tab and went dead on the others. A scrollspy (rather than
  // an IntersectionObserver band) also lets the last file win: a short trailing
  // file can never scroll far enough up to trip an observer trigger zone.
  useEffect(() => {
    const root = scrollRootRef.current;
    if (!root || files.length === 0) return;

    let raf = 0;
    const recompute = () => {
      raf = 0;
      if (lockedRef.current) return;
      const trigger = root.getBoundingClientRect().top + 100;
      let active = 0;
      for (let i = 0; i < files.length; i++) {
        const el = root.querySelector<HTMLElement>(`#file-${i}`);
        if (!el) continue;
        if (el.getBoundingClientRect().top <= trigger) active = i;
        else break;
      }
      // A short trailing file can't reach the trigger line; once the panel is
      // scrolled to the bottom, treat the last file as active.
      if (root.scrollTop + root.clientHeight >= root.scrollHeight - 2) {
        active = files.length - 1;
      }
      setActiveIndex(active);
    };
    const schedule = () => {
      if (!raf) raf = requestAnimationFrame(recompute);
    };
    const onScroll = () => {
      // While a manual selection holds, don't auto-pick. Each scroll event from
      // the click's smooth scroll just pushes the release back; once scrolling
      // has been quiet briefly, hand control back to the scrollspy as-is.
      if (lockedRef.current) {
        window.clearTimeout(releaseTimerRef.current);
        releaseTimerRef.current = window.setTimeout(() => {
          lockedRef.current = false;
        }, 150);
        return;
      }
      schedule();
    };

    recompute();
    root.addEventListener("scroll", onScroll, { passive: true });
    const ro = new ResizeObserver(schedule);
    ro.observe(root);
    return () => {
      root.removeEventListener("scroll", onScroll);
      ro.disconnect();
      if (raf) cancelAnimationFrame(raf);
      window.clearTimeout(releaseTimerRef.current);
    };
  }, [files.length, reviewId, scrollRootRef]);

  const jumpTo = (index: number) => {
    lockedRef.current = true;
    // Fallback for clicking a file that's already in view: no scroll fires, so
    // the scroll-settle handler never runs — release the lock after a beat.
    window.clearTimeout(releaseTimerRef.current);
    releaseTimerRef.current = window.setTimeout(() => {
      lockedRef.current = false;
    }, 700);
    setActiveIndex(index);
    scrollRootRef.current
      ?.querySelector(`#file-${index}`)
      ?.scrollIntoView({ behavior: "smooth", block: "start" });
  };

  // Publish the live nav state so ReviewView's keyboard handler can drive
  // file-nav off the same scrollspy (`jumpTo` keeps its lock/release behaviour).
  // Dep-less so it republishes after every render (activeIndex changes as the
  // user scrolls); nulled on unmount.
  useEffect(() => {
    if (!controlRef) return;
    controlRef.current = { activeIndex, fileCount: rows.length, jumpTo };
    return () => {
      controlRef.current = null;
    };
  });

  const tree = useMemo(() => buildTree(rows), [rows]);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const toggleDir = (path: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });

  const renderNodes = (nodes: TreeNode[], depth: number): ReactNode[] =>
    nodes.flatMap((node) => {
      const indent = { paddingLeft: 8 + depth * 12 };
      if (node.kind === "dir") {
        const open = !collapsed.has(node.path);
        return [
          <button
            key={`dir:${node.path}`}
            type="button"
            className="tree-row tree-dir"
            style={indent}
            onClick={() => toggleDir(node.path)}
            title={node.path}
          >
            <Chevron open={open} />
            <Icon name="folder" size={13} className="tree-folder" />
            <span className="tree-name">{node.name}</span>
          </button>,
          ...(open ? renderNodes(node.children, depth + 1) : []),
        ];
      }
      const { row } = node;
      return [
        <button
          key={`file:${row.index}`}
          type="button"
          className={`tree-row${row.index === activeIndex ? " active" : ""}${
            row.viewed ? " viewed" : ""
          }`}
          style={indent}
          onClick={() => jumpTo(row.index)}
          title={row.path}
        >
          <Icon name="file" size={13} className="tree-file" />
          <span className="tree-name">{node.name}</span>
          <span className="jump-meta">
            {row.count > 0 && <span className="jump-badge">{row.count}</span>}
            <span className="delta-add">+{row.add}</span>
            <span className="delta-del">−{row.del}</span>
          </span>
        </button>,
      ];
    });

  // ── Comments tab ──────────────────────────────────────────────
  const [activeTab, setActiveTab] = useState<"files" | "comments">("files");
  const [resolutionFilter, setResolutionFilter] = useState<ResolutionFilter>("all");
  const [sourceFilter, setSourceFilter] = useState<SourceFilter>("all");
  const [sortMode, setSortMode] = useState<SortMode>("file");

  const allComments = detail?.comments ?? [];

  const displayItems = useMemo((): DisplayItem[] => {
    const items: DisplayItem[] = [];

    // Build reply map for local comments
    const repliesByParent = new Map<number, typeof allComments>();
    for (const c of allComments) {
      if (c.parent_id != null) {
        const arr = repliesByParent.get(c.parent_id) ?? [];
        arr.push(c);
        repliesByParent.set(c.parent_id, arr);
      }
    }

    // Local root comments
    for (const c of allComments) {
      if (c.parent_id != null || isDraftReply(c)) continue;
      const rawReplies = repliesByParent.get(c.id) ?? [];
      rawReplies.sort(
        (a, b) => new Date(a.created_at).getTime() - new Date(b.created_at).getTime(),
      );
      items.push({
        key: `local:${c.id}`,
        filePath: c.file_path,
        line: c.line,
        startLine: c.start_line,
        isFileLevel: c.subject_type === "file",
        body: c.body,
        isResolved: c.resolved_at != null,
        createdAt: c.created_at,
        source: c.github_comment_id != null ? "published" : "draft",
        scrollId: String(c.id),
        scrollAttr: "comment-id",
        replies: rawReplies.map((r) => ({
          key: `local-reply:${r.id}`,
          body: r.body,
          createdAt: r.created_at,
          source: r.github_comment_id != null ? ("published" as const) : ("draft" as const),
        })),
      });
    }

    // GitHub PR threads (first comment is the root)
    for (const t of threads) {
      if (!t.path || t.comments.length === 0) continue;
      const [root, ...rest] = t.comments;
      items.push({
        key: `github:${t.id}`,
        filePath: t.path,
        line: t.line,
        startLine: t.startLine,
        isFileLevel: t.subjectType === "FILE",
        body: root.body,
        isResolved: t.isResolved,
        createdAt: root.createdAt,
        source: "github",
        scrollId: t.id,
        scrollAttr: "thread-id",
        replies: rest.map((r) => ({
          key: `gh-reply:${r.id}`,
          body: r.body,
          createdAt: r.createdAt,
          source: "github" as const,
        })),
      });
    }

    return items;
  }, [allComments, threads]);

  const filteredItems = useMemo(() => {
    return displayItems.filter((item) => {
      if (resolutionFilter === "unresolved" && item.isResolved) return false;
      if (resolutionFilter === "resolved" && !item.isResolved) return false;
      if (sourceFilter !== "all" && item.source !== sourceFilter) return false;
      return true;
    });
  }, [displayItems, resolutionFilter, sourceFilter]);

  const scrollToItem = (item: DisplayItem) => {
    const root = scrollRootRef.current;
    if (!root) return;
    const attr = item.scrollAttr === "comment-id" ? "data-comment-id" : "data-thread-id";
    const el = root.querySelector<HTMLElement>(`[${attr}="${item.scrollId}"]`);
    if (el) {
      el.scrollIntoView({ behavior: "smooth", block: "center" });
      el.classList.add("kb-flash");
      setTimeout(() => el.classList.remove("kb-flash"), 900);
    } else {
      // Comment is inside a collapsed (viewed) file — scroll to its file header
      const fileIdx = rows.findIndex((r) => r.path === item.filePath);
      if (fileIdx >= 0) jumpTo(fileIdx);
    }
  };

  const renderThread = (item: DisplayItem) => {
    const loc = item.isFileLevel
      ? "File"
      : item.line == null
        ? "—"
        : item.startLine != null && item.startLine !== item.line
          ? `L${item.startLine}–${item.line}`
          : `L${item.line}`;
    return (
      <div key={item.key} className={`comment-list-thread${item.isResolved ? " resolved" : ""}`}>
        <button
          type="button"
          className="comment-list-row"
          onClick={() => scrollToItem(item)}
          title={`${item.filePath}${item.line != null ? `:${item.line}` : ""}`}
        >
          <span className="comment-list-loc">{loc}</span>
          <span className="comment-list-body">{summaryLine(item.body)}</span>
          <span className="comment-list-meta">
            {item.source === "github" && (
              <span className="comment-list-source-gh">GH</span>
            )}
            {item.isResolved && (
              <Icon name="check" size={11} className="comment-list-resolved-icon" />
            )}
            <span className="comment-list-time">{timeAgo(item.createdAt)}</span>
          </span>
        </button>
        {item.replies.map((r) => (
          <button
            key={r.key}
            type="button"
            className="comment-list-reply"
            onClick={() => scrollToItem(item)}
            title={r.body}
          >
            <span className="comment-list-reply-body">{summaryLine(r.body)}</span>
            {r.source === "github" && (
              <span className="comment-list-source-gh">GH</span>
            )}
            <span className="comment-list-time">{timeAgo(r.createdAt)}</span>
          </button>
        ))}
      </div>
    );
  };

  const renderCommentList = (): ReactNode => {
    if (filteredItems.length === 0) {
      return (
        <p className="comment-list-empty">
          {displayItems.length === 0 ? "No comments yet." : "No matching comments."}
        </p>
      );
    }

    if (sortMode === "time") {
      const effectiveTime = (item: DisplayItem) => {
        const last = item.replies[item.replies.length - 1];
        return new Date(last ? last.createdAt : item.createdAt).getTime();
      };
      const sorted = [...filteredItems].sort((a, b) => effectiveTime(b) - effectiveTime(a));
      return sorted.map(renderThread);
    }

    // Group by file, ordered by file index in the diff
    const pathOrder = new Map(rows.map((r) => [r.path, r.index]));
    const groups = new Map<string, DisplayItem[]>();
    for (const item of filteredItems) {
      const arr = groups.get(item.filePath) ?? [];
      arr.push(item);
      groups.set(item.filePath, arr);
    }
    const sortedPaths = [...groups.keys()].sort((a, b) => {
      const ai = pathOrder.get(a) ?? Infinity;
      const bi = pathOrder.get(b) ?? Infinity;
      return ai !== bi ? ai - bi : a.localeCompare(b);
    });

    return sortedPaths.map((path) => {
      const groupItems = (groups.get(path) ?? []).sort((a, b) => {
        if (a.isFileLevel && !b.isFileLevel) return -1;
        if (!a.isFileLevel && b.isFileLevel) return 1;
        return (a.line ?? 0) - (b.line ?? 0);
      });
      const fileName = path.split("/").pop() ?? path;
      return (
        <div key={path} className="comment-list-group">
          <div className="comment-list-file" title={path}>
            {fileName}
          </div>
          {groupItems.map(renderThread)}
        </div>
      );
    });
  };

  const hasGithubThreads = threads.length > 0;

  return (
    <nav className={`jump-list${paneCollapsed ? " jump-list--collapsed" : ""}`}>
      {paneCollapsed ? (
        <button
          className="btn btn-sm btn-ghost sidebar-toggle sidebar-expand-btn"
          title="Show file list (b)"
          onClick={onToggle}
        >
          <Icon name="menu" size={14} />
        </button>
      ) : (
        <>
          <div className="sidebar-tabs">
            <button
              type="button"
              className={`sidebar-tab${activeTab === "files" ? " active" : ""}`}
              onClick={() => setActiveTab("files")}
            >
              Files ({rows.length})
            </button>
            <button
              type="button"
              className={`sidebar-tab${activeTab === "comments" ? " active" : ""}`}
              onClick={() => setActiveTab("comments")}
            >
              Comments ({displayItems.length})
            </button>
            {onToggle && (
              <button
                className="btn btn-sm btn-ghost sidebar-toggle"
                title="Hide sidebar (b)"
                onClick={onToggle}
              >
                <Icon name="x" size={12} />
              </button>
            )}
          </div>

          {activeTab === "files" ? (
            <div className="jump-list-content">
              {renderNodes(tree, 0)}
            </div>
          ) : (
            <>
              <div className="comment-filters">
                <div className="comment-filter-group">
                  {(["all", "unresolved", "resolved"] as ResolutionFilter[]).map((f) => (
                    <button
                      key={f}
                      type="button"
                      className={`comment-filter-btn${resolutionFilter === f ? " active" : ""}`}
                      onClick={() => setResolutionFilter(f)}
                    >
                      {f === "all" ? "All" : f === "unresolved" ? "Open" : "Resolved"}
                    </button>
                  ))}
                </div>
                {hasGithubThreads && (
                  <div className="comment-filter-group">
                    {(["all", "draft", "published", "github"] as SourceFilter[]).map((s) => (
                      <button
                        key={s}
                        type="button"
                        className={`comment-filter-btn${sourceFilter === s ? " active" : ""}`}
                        onClick={() => setSourceFilter(s)}
                      >
                        {s === "all" ? "All" : s === "draft" ? "Draft" : s === "published" ? "Pub" : "GH"}
                      </button>
                    ))}
                  </div>
                )}
                <div className="comment-filter-group">
                  {(["file", "time"] as SortMode[]).map((s) => (
                    <button
                      key={s}
                      type="button"
                      className={`comment-filter-btn${sortMode === s ? " active" : ""}`}
                      onClick={() => setSortMode(s)}
                    >
                      {s === "file" ? "By file" : "By time"}
                    </button>
                  ))}
                </div>
              </div>
              <div className="jump-list-content">
                {renderCommentList()}
              </div>
            </>
          )}
        </>
      )}
    </nav>
  );
}
