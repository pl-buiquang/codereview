import type { QueryKey } from "@tanstack/react-query";
import type { GitChangeKind } from "./types";

export const GIT_CHANGED_EVENT = "git-changed";

export interface GitChangedPayload {
  repo_id: number;
  kinds: GitChangeKind[];
}

const KEYS_BY_KIND: Record<GitChangeKind, string[]> = {
  index: ["wip"],
  head: ["wip", "refs", "graph", "worktrees", "branches", "review-live"],
  refs: ["refs", "graph", "branches", "review-live"],
  fetch: ["refs", "graph", "branches", "repo-sync"],
};

/** Query-key prefixes (all repo-scoped) to invalidate for a watcher event. */
export function gitChangedQueryKeys({ repo_id, kinds }: GitChangedPayload): QueryKey[] {
  const prefixes = new Set(kinds.flatMap((k) => KEYS_BY_KIND[k] ?? []));
  return [...prefixes].map((p) => [p, repo_id]);
}
