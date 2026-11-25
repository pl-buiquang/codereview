import { isAiTag, willPublish, type AiSubtag, type CommentTag } from "./commentTag";
import type { Comment, PrThread } from "./types";

export type ResolutionFilter = "all" | "unresolved" | "resolved";
/** "publish": only what the next publish would send (publish toggle on, not yet posted). */
export type PublishFilter = "all" | "publish";
export type SourceFilter = "all" | "draft" | "published" | "github" | "ai" | AiSubtag | "local";

export interface CommentFilter {
  resolution: ResolutionFilter;
  source: SourceFilter;
  publish: PublishFilter;
}

export const DEFAULT_COMMENT_FILTER: CommentFilter = {
  resolution: "all",
  source: "all",
  publish: "all",
};

export function isCommentFilterActive(f: CommentFilter): boolean {
  return f.resolution !== "all" || f.source !== "all" || f.publish !== "all";
}

/** What the filters look at, shared by local comments and GitHub threads. */
export interface FilterableComment {
  isResolved: boolean;
  source: "draft" | "published" | "github";
  tag: CommentTag | null;
  /** The next publish sends it: a local root, not posted yet, publish toggle on. */
  toPublish: boolean;
}

export function matchesCommentFilter(item: FilterableComment, f: CommentFilter): boolean {
  if (f.resolution === "unresolved" && item.isResolved) return false;
  if (f.resolution === "resolved" && !item.isResolved) return false;
  if (f.publish === "publish" && !item.toPublish) return false;
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
    toPublish: isToPublish(root),
  };
}

/** Mirrors the publish count: root, not a GitHub draft reply, not posted, toggle on. */
export function isToPublish(c: Comment): boolean {
  return (
    c.parent_id == null &&
    c.github_thread_root_id == null &&
    c.github_comment_id == null &&
    willPublish(c)
  );
}

export function githubThreadItem(t: Pick<PrThread, "isResolved">): FilterableComment {
  return { isResolved: t.isResolved, source: "github", tag: null, toPublish: false };
}
