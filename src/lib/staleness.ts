import type { Comment } from "./types";

/** The pin SHA relevant to this comment's side (LEFT → base, RIGHT → head). */
export function anchorPin(c: Comment): string | null {
  return c.side === "LEFT" ? c.anchored_base_sha : c.anchored_head_sha;
}

/** True when the comment was anchored to a SHA its side has since moved past,
 *  so its line may no longer point at the code it was written against. LEFT line
 *  numbers live in base coordinates, so the head is irrelevant to them (and vice
 *  versa). A NULL pin or NULL current SHA is never outdated. */
export function isCommentOutdated(
  c: Comment,
  baseSha: string | null,
  headSha: string | null,
): boolean {
  const pin = anchorPin(c);
  const current = c.side === "LEFT" ? baseSha : headSha;
  return !!pin && !!current && pin !== current;
}
