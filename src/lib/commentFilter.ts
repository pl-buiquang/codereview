import { isAiTag, type AiSubtag, type CommentTag } from "./commentTag";
import type { Comment, PrThread } from "./types";

export type ResolutionFilter = "all" | "unresolved" | "resolved";
export type SourceFilter = "all" | "draft" | "published" | "github" | "ai" | AiSubtag | "local";

export interface CommentFilter {
  resolution: ResolutionFilter;
  source: SourceFilter;
}

export const DEFAULT_COMMENT_FILTER: CommentFilter = { resolution: "all", source: "all" };

export function isCommentFilterActive(f: CommentFilter): boolean {
  return f.resolution !== "all" || f.source !== "all";
}

/** What the filters look at, shared by local comments and GitHub threads. */
export interface FilterableComment {
  isResolved: boolean;
  source: "draft" | "published" | "github";
  tag: CommentTag | null;
}

export function matchesCommentFilter(item: FilterableComment, f: CommentFilter): boolean {
  if (f.resolution === "unresolved" && item.isResolved) return false;
  if (f.resolution === "resolved" && !item.isResolved) return false;
  const s = f.source;
  if (s === "all") return true;
  if (s === "ai") return isAiTag(item.tag);
  if (s === "local" || isAiTag(s as CommentTag)) return item.tag === s;
  if (s === "draft") return item.source === "draft" && item.tag == null;
  return item.source === s;
}

export function localCommentItem(root: Comment): FilterableComment {
  return {
    isResolved: root.resolved_at != null,
    source: root.github_comment_id != null ? "published" : "draft",
    tag: root.tag,
  };
}

export function githubThreadItem(t: Pick<PrThread, "isResolved">): FilterableComment {
  return { isResolved: t.isResolved, source: "github", tag: null };
}
