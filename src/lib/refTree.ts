import type { RefInfo } from "./types";

export type RefSort = "updated" | "name";

export type RefTreeNode =
  | { type: "folder"; path: string; label: string; updated: number; count: number; children: RefTreeNode[] }
  | { type: "leaf"; label: string; ref: RefInfo };

type MutableFolder = Extract<RefTreeNode, { type: "folder" }>;

const nodeName = (n: RefTreeNode) => n.label.toLowerCase();
const nodeUpdated = (n: RefTreeNode) => (n.type === "folder" ? n.updated : n.ref.updated_at);

function sortNodes(nodes: RefTreeNode[], sort: RefSort): RefTreeNode[] {
  nodes.sort((a, b) => {
    if (sort === "updated") return nodeUpdated(b) - nodeUpdated(a) || nodeName(a).localeCompare(nodeName(b));
    if (a.type !== b.type) return a.type === "folder" ? -1 : 1;
    return nodeName(a).localeCompare(nodeName(b));
  });
  for (const n of nodes) if (n.type === "folder") sortNodes(n.children, sort);
  return nodes;
}

/** Groups refs into folders by `/`-separated name segments; a folder's `updated` is its newest ref. */
export function buildRefTree(refs: RefInfo[], sort: RefSort): RefTreeNode[] {
  const root: MutableFolder = { type: "folder", path: "", label: "", updated: 0, count: 0, children: [] };
  for (const ref of refs) {
    const parts = ref.name.split("/").filter(Boolean);
    let folder = root;
    for (let i = 0; i < parts.length - 1; i++) {
      const path = parts.slice(0, i + 1).join("/");
      let next = folder.children.find((c): c is MutableFolder => c.type === "folder" && c.path === path);
      if (!next) {
        next = { type: "folder", path, label: parts[i], updated: 0, count: 0, children: [] };
        folder.children.push(next);
      }
      next.updated = Math.max(next.updated, ref.updated_at);
      next.count += 1;
      folder = next;
    }
    folder.children.push({ type: "leaf", label: parts[parts.length - 1] ?? ref.name, ref });
  }
  return sortNodes(root.children, sort);
}

/** Folder paths that are ancestors of `name` (e.g. "a/b/c" → ["a", "a/b"]). */
export function ancestorPaths(name: string): string[] {
  const parts = name.split("/").filter(Boolean);
  return parts.slice(0, -1).map((_, i) => parts.slice(0, i + 1).join("/"));
}
