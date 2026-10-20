import { describe, it, expect } from "vitest";
import {
  DEFAULT_COMMENT_FILTER,
  isCommentFilterActive,
  matchesCommentFilter,
  type CommentFilter,
  type FilterableComment,
} from "./commentFilter";

const item = (over: Partial<FilterableComment> = {}): FilterableComment => ({
  isResolved: false,
  source: "draft",
  tag: null,
  ...over,
});
const f = (over: Partial<CommentFilter>): CommentFilter => ({ ...DEFAULT_COMMENT_FILTER, ...over });

describe("matchesCommentFilter", () => {
  it("keeps everything by default", () => {
    expect(matchesCommentFilter(item(), DEFAULT_COMMENT_FILTER)).toBe(true);
    expect(isCommentFilterActive(DEFAULT_COMMENT_FILTER)).toBe(false);
  });

  it("filters by resolution", () => {
    expect(matchesCommentFilter(item({ isResolved: true }), f({ resolution: "unresolved" }))).toBe(false);
    expect(matchesCommentFilter(item({ isResolved: false }), f({ resolution: "resolved" }))).toBe(false);
    expect(matchesCommentFilter(item({ isResolved: true }), f({ resolution: "resolved" }))).toBe(true);
  });

  it("draft excludes tagged comments", () => {
    expect(matchesCommentFilter(item({ tag: "ai:review" }), f({ source: "draft" }))).toBe(false);
    expect(matchesCommentFilter(item({ tag: "local" }), f({ source: "draft" }))).toBe(false);
    expect(matchesCommentFilter(item(), f({ source: "draft" }))).toBe(true);
  });

  it("ai matches any ai tag, a subtag only its own", () => {
    expect(matchesCommentFilter(item({ tag: "ai:codex" }), f({ source: "ai" }))).toBe(true);
    expect(matchesCommentFilter(item({ tag: "ai:codex" }), f({ source: "ai:review" }))).toBe(false);
    expect(matchesCommentFilter(item({ tag: "ai:codex" }), f({ source: "ai:codex" }))).toBe(true);
    expect(matchesCommentFilter(item({ tag: "local" }), f({ source: "ai" }))).toBe(false);
  });

  it("local and source filters", () => {
    expect(matchesCommentFilter(item({ tag: "local" }), f({ source: "local" }))).toBe(true);
    expect(matchesCommentFilter(item({ source: "github" }), f({ source: "github" }))).toBe(true);
    expect(matchesCommentFilter(item({ source: "published" }), f({ source: "github" }))).toBe(false);
    expect(isCommentFilterActive(f({ source: "github" }))).toBe(true);
  });
});
