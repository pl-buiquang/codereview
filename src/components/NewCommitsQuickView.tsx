import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import { QuickDiffPane } from "./QuickViewPane";

/**
 * Slide-out previewing what changed in a PR since a review's pin (the rebase-aware
 * interdiff), with nothing stored. Create review starts the real "since" review,
 * pinned at exactly the commits this diff was computed against.
 */
export function NewCommitsQuickView({
  reviewId,
  headSha,
  busy,
  onClose,
  onCreate,
}: {
  reviewId: number;
  /** The review's pinned head, so the preview refetches after an Update review. */
  headSha: string | null;
  busy: boolean;
  onClose: () => void;
  onCreate: (pin: { baseSha: string; headSha: string }) => void;
}) {
  const query = useQuery({
    queryKey: ["quick-interdiff", reviewId, headSha],
    queryFn: () => api.quickInterdiff(reviewId),
    staleTime: 60_000,
    retry: false,
  });
  const data = query.data;
  return (
    <QuickDiffPane
      title={
        <>
          Changes since this review
          {data?.rebased && <span className="muted"> (rebased)</span>}
          {data?.approximate && (
            <span
              className="muted"
              title="The rebase could not be replayed cleanly, so this diff may include upstream changes or miss some context."
            >
              {" "}
              ⚠ approximate
            </span>
          )}
        </>
      }
      titleText="What changed in the PR since this review's commits"
      diffText={data?.diff}
      isLoading={query.isLoading}
      error={query.isError ? query.error : null}
      onClose={onClose}
      actions={
        <button
          className="btn btn-sm btn-primary"
          disabled={data == null || busy}
          onClick={() => data && onCreate({ baseSha: data.baseSha, headSha: data.headSha })}
          title="Create a real review on this diff"
        >
          {busy ? "Creating…" : "Create review"}
        </button>
      }
    />
  );
}
