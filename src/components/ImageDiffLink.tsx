import { openImageDiffs, useImageDiffStatus } from "../lib/imageDiff";

/** The link under an image file's "not shown" note: opens the image diffs panel on that file,
 *  generating them first when there are none. */
export function ImageDiffLink({ reviewId, path }: { reviewId: number; path: string }) {
  const status = useImageDiffStatus(reviewId);
  const state = status.data?.state;
  const label =
    state === "ready" || state === "stale"
      ? "View image diff"
      : state === "running"
        ? "Image diffs are generating…"
        : "Generate image diffs";
  return (
    <button className="btn btn-sm btn-ghost" disabled={status.isLoading} onClick={() => openImageDiffs(reviewId, state, path)}>
      {label}
    </button>
  );
}
