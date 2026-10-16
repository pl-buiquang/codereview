import { queryOptions } from "@tanstack/react-query";
import { api } from "./api";
import type { ReviewDetail } from "./types";

type DiffKeyDetail = { review: Pick<ReviewDetail["review"], "base_sha" | "head_sha">; target: Pick<ReviewDetail["target"], "id" | "three_dot"> };

/** A diff is immutable for a given pin (base, head) and three_dot, and those
 *  are all in the key, so it never goes stale; moving the pin changes the key. */
export function reviewDiffQuery(reviewId: number, detail: DiffKeyDetail | undefined) {
  return queryOptions({
    queryKey: [
      "review-diff",
      reviewId,
      detail?.target.id,
      detail?.review.base_sha,
      detail?.review.head_sha,
      detail?.target.three_dot,
    ],
    queryFn: () => api.reviewDiff(reviewId),
    enabled: detail != null,
    staleTime: Infinity,
  });
}

export function headFileSourceQuery(reviewId: number, filePath: string, headSha: string | null) {
  return queryOptions({
    queryKey: ["file-source", reviewId, filePath, "RIGHT", headSha],
    queryFn: () => api.fileSource(reviewId, filePath, "RIGHT"),
    staleTime: Infinity,
  });
}
