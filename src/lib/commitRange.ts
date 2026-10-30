import type { GraphCommit } from "./types";

/** A click-then-shift-click selection over a newest-first commit list. */
export interface CommitSelection {
  anchor: number;
  focus: number;
}

/** Plain click selects one commit; shift-click extends from the anchor. */
export function selectCommit(prev: CommitSelection | null, index: number, shift: boolean): CommitSelection {
  return shift && prev ? { anchor: prev.anchor, focus: index } : { anchor: index, focus: index };
}

export interface CommitRange {
  /** The oldest selected commit's first parent (null for a root commit). */
  base: string | null;
  /** The newest selected commit. */
  head: string;
  count: number;
}

/** The diff range a selection covers: `parent(oldest)..newest`. `commits` is newest-first. */
export function commitRange(commits: GraphCommit[], sel: CommitSelection): CommitRange {
  const lo = Math.min(sel.anchor, sel.focus);
  const hi = Math.max(sel.anchor, sel.focus);
  return {
    base: commits[hi]?.parents[0] ?? null,
    head: commits[lo].sha,
    count: hi - lo + 1,
  };
}

export function isSelected(sel: CommitSelection | null, index: number): boolean {
  return sel != null && index >= Math.min(sel.anchor, sel.focus) && index <= Math.max(sel.anchor, sel.focus);
}
