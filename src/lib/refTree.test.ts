import { describe, expect, it } from "vitest";
import { ancestorPaths, buildRefTree, type RefTreeNode } from "./refTree";
import type { RefInfo } from "./types";

const ref = (name: string, updated_at: number): RefInfo => ({
  name,
  kind: "local",
  sha: "a".repeat(40),
  is_head: false,
  updated_at,
});

const labels = (nodes: RefTreeNode[]) => nodes.map((n) => (n.type === "folder" ? `${n.label}/` : n.label));

describe("buildRefTree", () => {
  const refs = [ref("main", 10), ref("pbq/a", 30), ref("pbq/deep/b", 50), ref("fix", 20), ref("dep/x", 5)];

  it("nests by slash and aggregates folder count and recency", () => {
    const tree = buildRefTree(refs, "name");
    const pbq = tree.find((n) => n.type === "folder" && n.path === "pbq");
    expect(pbq).toMatchObject({ count: 2, updated: 50 });
    if (pbq?.type !== "folder") throw new Error("expected folder");
    expect(labels(pbq.children)).toEqual(["deep/", "a"]);
    const deep = pbq.children[0];
    expect(deep.type === "folder" && deep.path).toBe("pbq/deep");
  });

  it("sorts by name with folders first", () => {
    expect(labels(buildRefTree(refs, "name"))).toEqual(["dep/", "pbq/", "fix", "main"]);
  });

  it("sorts by most recently updated, folders ranked by their newest ref", () => {
    expect(labels(buildRefTree(refs, "updated"))).toEqual(["pbq/", "fix", "main", "dep/"]);
  });
});

describe("ancestorPaths", () => {
  it("lists every enclosing folder", () => {
    expect(ancestorPaths("a/b/c")).toEqual(["a", "a/b"]);
    expect(ancestorPaths("main")).toEqual([]);
  });
});
