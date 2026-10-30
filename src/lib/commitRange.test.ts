import { describe, it, expect } from "vitest";
import { commitRange, isSelected, selectCommit } from "./commitRange";
import type { GraphCommit } from "./types";

const commit = (sha: string, parent?: string) => ({ sha, parents: parent ? [parent] : [] }) as GraphCommit;
// Newest first: c3 -> c2 -> c1 -> (root)
const commits = [commit("c3", "c2"), commit("c2", "c1"), commit("c1")];

describe("commit range selection", () => {
  it("a click selects one commit, shift-click extends from the anchor", () => {
    const one = selectCommit(null, 1, false);
    expect(one).toEqual({ anchor: 1, focus: 1 });
    expect(selectCommit(one, 0, true)).toEqual({ anchor: 1, focus: 0 });
    expect(selectCommit(null, 2, true)).toEqual({ anchor: 2, focus: 2 });
    expect(selectCommit(one, 2, false)).toEqual({ anchor: 2, focus: 2 });
  });

  it("maps a selection to parent(oldest)..newest regardless of click order", () => {
    expect(commitRange(commits, { anchor: 0, focus: 1 })).toEqual({ base: "c1", head: "c3", count: 2 });
    expect(commitRange(commits, { anchor: 1, focus: 0 })).toEqual({ base: "c1", head: "c3", count: 2 });
    expect(commitRange(commits, { anchor: 1, focus: 1 })).toEqual({ base: "c1", head: "c2", count: 1 });
  });

  it("has no base when the oldest commit is a root", () => {
    expect(commitRange(commits, { anchor: 0, focus: 2 }).base).toBeNull();
  });

  it("isSelected covers the inclusive range", () => {
    const sel = { anchor: 2, focus: 1 };
    expect([0, 1, 2].map((i) => isSelected(sel, i))).toEqual([false, true, true]);
    expect(isSelected(null, 0)).toBe(false);
  });
});
