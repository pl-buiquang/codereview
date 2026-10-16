import { queryOptions } from "@tanstack/react-query";
import { api } from "./api";
import type { Target } from "./types";

type DiffKeyTarget = Pick<Target, "id" | "base_sha" | "head_sha" | "three_dot">;

/** A diff is immutable for a given (base, head, three_dot), and those are all
 *  in the key, so it never goes stale; a moved head or base changes the key. */
export function reviewDiffQuery(reviewId: number, target: DiffKeyTarget | undefined) {
  return queryOptions({
    queryKey: [
      "review-diff",
      reviewId,
      target?.id,
      target?.base_sha,
      target?.head_sha,
      target?.three_dot,
    ],
    queryFn: () => api.reviewDiff(reviewId),
    enabled: target != null,
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
