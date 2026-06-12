import { describe, it, expect } from "vitest";
import { anchorPin, isCommentOutdated } from "./staleness";
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

describe("isCommentOutdated", () => {
  it("flags a RIGHT comment whose head pin no longer matches the head", () => {
    const c = comment({ id: 1, side: "RIGHT", anchored_head_sha: "old" });
    expect(isCommentOutdated(c, "base", "new")).toBe(true);
  });

  it("flags a LEFT comment whose base pin no longer matches the base, regardless of head", () => {
    const c = comment({ id: 2, side: "LEFT", anchored_base_sha: "oldbase" });
    // Head matches a head pin if it existed, but LEFT ignores the head entirely.
    expect(isCommentOutdated(c, "newbase", "anyhead")).toBe(true);
    expect(isCommentOutdated(c, "newbase", "oldbase")).toBe(true);
  });

  it("does not flag a LEFT comment on the current base even when the head moved", () => {
    const c = comment({
      id: 3,
      side: "LEFT",
      anchored_base_sha: "base",
      anchored_head_sha: "stalehead",
    });
    expect(isCommentOutdated(c, "base", "freshhead")).toBe(false);
  });

  it("does not flag a RIGHT comment on the current head even when the base moved", () => {
    const c = comment({ id: 4, side: "RIGHT", anchored_head_sha: "head" });
    expect(isCommentOutdated(c, "movedbase", "head")).toBe(false);
  });

  it("is never outdated with a NULL pin or NULL current sha", () => {
    const rightNoPin = comment({ id: 5, side: "RIGHT", anchored_head_sha: null });
    const leftNoPin = comment({ id: 6, side: "LEFT", anchored_base_sha: null });
    expect(isCommentOutdated(rightNoPin, "base", "head")).toBe(false);
    expect(isCommentOutdated(leftNoPin, "base", "head")).toBe(false);

    const rightPinned = comment({ id: 7, side: "RIGHT", anchored_head_sha: "head" });
    const leftPinned = comment({ id: 8, side: "LEFT", anchored_base_sha: "base" });
    // Current sha unknown → cannot say it moved.
    expect(isCommentOutdated(rightPinned, null, null)).toBe(false);
    expect(isCommentOutdated(leftPinned, null, null)).toBe(false);
  });
});

describe("anchorPin", () => {
  it("picks the side-correct column", () => {
    const right = comment({
      id: 1,
      side: "RIGHT",
      anchored_head_sha: "h",
      anchored_base_sha: "b",
    });
    const left = comment({
      id: 2,
      side: "LEFT",
      anchored_head_sha: "h",
      anchored_base_sha: "b",
    });
    expect(anchorPin(right)).toBe("h");
    expect(anchorPin(left)).toBe("b");
  });
});
