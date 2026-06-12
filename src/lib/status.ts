import type { Review } from "./types";

/** Human label for a review status badge. */
export function statusLabel(status: Review["status"]): string {
  return status === "published_pending" ? "pending on GitHub" : status;
}
