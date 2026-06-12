import type { Comment } from "./types";

export interface CommentThread {
  root: Comment;
  replies: Comment[]; // created_at asc, id as tiebreak
}

/**
 * Group flat comment rows into root threads. A reply whose parent is not in the
 * input is promoted to a root (defensive — never drop data). Root order follows
 * the input order; replies are sorted by `created_at` then `id`.
 */
export function groupThreads(comments: Comment[]): CommentThread[] {
  const byId = new Map<number, Comment>();
  for (const c of comments) byId.set(c.id, c);

  const threads: CommentThread[] = [];
  const indexByRootId = new Map<number, number>();
  const pendingReplies: Comment[] = [];

  for (const c of comments) {
    if (c.parent_id != null && byId.has(c.parent_id)) {
      pendingReplies.push(c);
      continue;
    }
    // A root, or an orphaned reply promoted to root.
    indexByRootId.set(c.id, threads.length);
    threads.push({ root: c, replies: [] });
  }

  for (const r of pendingReplies) {
    const idx = indexByRootId.get(r.parent_id as number);
    if (idx == null) {
      // Parent exists in the map but is itself a reply (shouldn't happen given
      // the backend's one-level contract) — promote defensively.
      indexByRootId.set(r.id, threads.length);
      threads.push({ root: r, replies: [] });
      continue;
    }
    threads[idx].replies.push(r);
  }

  for (const t of threads) {
    t.replies.sort(
      (a, b) => a.created_at.localeCompare(b.created_at) || a.id - b.id,
    );
  }

  return threads;
}
