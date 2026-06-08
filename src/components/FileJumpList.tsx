import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import { useQuery } from "@tanstack/react-query";
import { parseDiff } from "react-diff-view";
import { api } from "../lib/api";
import { countChanges, fileDisplayPath } from "../lib/diff";

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

function FolderIcon() {
  return (
    <svg
      className="tree-folder"
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M4 20a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2Z" />
    </svg>
  );
}

export function FileJumpList({
  reviewId,
  scrollRootRef,
}: {
  reviewId: number;
  scrollRootRef: RefObject<HTMLElement | null>;
}) {
  const detailQuery = useQuery({
    queryKey: ["review", reviewId],
    queryFn: () => api.getReview(reviewId),
    enabled: reviewId != null,
  });
  const detail = detailQuery.data;

  const diffQuery = useQuery({
    queryKey: ["review-diff", reviewId, detail?.target.id],
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
            <FolderIcon />
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
          <span className="tree-name">{node.name}</span>
          <span className="jump-meta">
            {row.count > 0 && <span className="jump-badge">{row.count}</span>}
            <span className="add">+{row.add}</span>
            <span className="del">−{row.del}</span>
          </span>
        </button>,
      ];
    });

  return (
    <nav className="jump-list">
      <div className="jump-list-header">Files ({rows.length})</div>
      {renderNodes(tree, 0)}
    </nav>
  );
}
