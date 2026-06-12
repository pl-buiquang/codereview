import { describe, it, expect } from "vitest";
import { groupThreads } from "./threads";
import type { Comment } from "./types";

function comment(over: Partial<Comment> & { id: number }): Comment {
  return {
    review_id: 1,
    file_path: "a.ts",
    subject_type: "line",
    origin: "diff",
    side: "RIGHT",
    line: 5,
    start_line: null,
    diff_hunk: null,
    body: `body ${over.id}`,
    parent_id: null,
    anchored_head_sha: null,
    anchored_base_sha: null,
    github_comment_id: null,
    resolved_at: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ...over,
  };
}

describe("groupThreads", () => {
  it("nests replies under their root and sorts by created_at then id", () => {
    const root = comment({ id: 1 });
    const r2 = comment({ id: 2, parent_id: 1, created_at: "2026-01-02T00:00:00Z" });
    const r3 = comment({ id: 3, parent_id: 1, created_at: "2026-01-01T12:00:00Z" });
    // r3 is earlier than r2 despite the higher id; it should sort first.
    const threads = groupThreads([root, r2, r3]);
    expect(threads).toHaveLength(1);
    expect(threads[0].root.id).toBe(1);
    expect(threads[0].replies.map((c) => c.id)).toEqual([3, 2]);
  });

  it("uses id as the tiebreak when created_at matches", () => {
    const root = comment({ id: 1 });
    const a = comment({ id: 5, parent_id: 1, created_at: "2026-02-01T00:00:00Z" });
    const b = comment({ id: 4, parent_id: 1, created_at: "2026-02-01T00:00:00Z" });
    const threads = groupThreads([root, a, b]);
    expect(threads[0].replies.map((c) => c.id)).toEqual([4, 5]);
  });

  it("promotes a reply whose parent is missing to a root", () => {
    const orphanReply = comment({ id: 9, parent_id: 42 });
    const threads = groupThreads([orphanReply]);
    expect(threads).toHaveLength(1);
    expect(threads[0].root.id).toBe(9);
    expect(threads[0].replies).toHaveLength(0);
  });

  it("preserves root input order", () => {
    const c2 = comment({ id: 2 });
    const c1 = comment({ id: 1 });
    const c3 = comment({ id: 3 });
    const threads = groupThreads([c2, c1, c3]);
    expect(threads.map((t) => t.root.id)).toEqual([2, 1, 3]);
  });

  it("round-trips a flat list with no replies", () => {
    const flat = [comment({ id: 1 }), comment({ id: 2 }), comment({ id: 3 })];
    const threads = groupThreads(flat);
    expect(threads).toHaveLength(3);
    expect(threads.every((t) => t.replies.length === 0)).toBe(true);
  });
});
