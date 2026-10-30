import { useMemo, useState, type ReactNode } from "react";
import type { FileCategory } from "../lib/fileCategory";
import { Icon, type IconName } from "./icons";

const FILE_ICON: Record<string, IconName> = {
  add: "file-add",
  delete: "file-del",
  rename: "file-move",
  copy: "file-move",
  modify: "file-edit",
};

export interface FileRow {
  index: number;
  path: string;
  add: number;
  del: number;
  count: number;
  viewed: boolean;
  category: FileCategory;
  /** react-diff-view's change type: add / delete / rename / copy / modify. */
  status: string;
}

type TreeNode =
  | { kind: "file"; name: string; row: FileRow }
  | { kind: "dir"; name: string; path: string; children: TreeNode[] };

/** Group flat file rows into a directory tree, GitHub-style: directory chains
 *  with a single child are collapsed into one node (`src/components`). Children
 *  keep their diff insertion order — never re-sorted — so the tree reads in the
 *  exact same order as the diff pane (git emits paths in byte-wise/C order, which
 *  is contiguous per directory, so first-seen order reproduces it). A locale sort
 *  would diverge on uppercase and dotted names. */
function buildTree(rows: FileRow[]): TreeNode[] {
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

/** Collapsed-directory state, owned by the caller so it survives the tree unmounting
 *  (e.g. switching sidebar tabs). */
export function useTreeCollapse() {
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const toggleDir = (path: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  return { collapsed, toggleDir };
}

/** The review's file tree: collapsible directories, per-file +/- deltas, comment badges. */
export function FileTree({
  rows,
  activeIndex,
  collapsed,
  onToggleDir,
  onPick,
}: {
  rows: FileRow[];
  activeIndex: number;
  collapsed: Set<string>;
  onToggleDir: (path: string) => void;
  onPick: (index: number) => void;
}) {
  const tree = useMemo(() => buildTree(rows), [rows]);

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
            onClick={() => onToggleDir(node.path)}
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
          onClick={() => onPick(row.index)}
          title={row.path}
        >
          <Icon name={FILE_ICON[row.status] ?? "file-edit"} size={13} className={`tree-file tree-file-${row.status}`} />
          <span className="tree-name">{node.name}</span>
          <span className="jump-meta">
            {row.count > 0 && <span className="jump-badge">{row.count}</span>}
            <span className="delta-add">+{row.add}</span>
            <span className="delta-del">−{row.del}</span>
          </span>
        </button>,
      ];
    });

  return <>{renderNodes(tree, 0)}</>;
}
