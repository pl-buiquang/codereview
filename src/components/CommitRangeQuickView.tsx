import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import { shortSha } from "./graph/CommitPanel";
import { QuickDiffPane } from "./QuickViewPane";

/**
 * Slide-out previewing the diff of a commit range picked from a PR's commit list, with
 * nothing stored. Create review starts a local review on exactly that range.
 */
export function CommitRangeQuickView({
  repoId,
  base,
  head,
  count,
  busy,
  onClose,
  onCreate,
}: {
  repoId: number;
  base: string;
  head: string;
  count: number;
  busy: boolean;
  onClose: () => void;
  onCreate: () => void;
}) {
  const query = useQuery({
    queryKey: ["quick-commit-range", repoId, base, head],
    queryFn: () => api.diffRefs(repoId, base, head, false),
    staleTime: 60_000,
    retry: false,
  });
  return (
    <QuickDiffPane
      title={
        <>
          <span className="mono">
            {shortSha(base)}..{shortSha(head)}
          </span>{" "}
          {count} {count === 1 ? "commit" : "commits"}
        </>
      }
      titleText="Changes of the selected commits"
      diffText={query.data}
      isLoading={query.isLoading}
      error={query.isError ? query.error : null}
      onClose={onClose}
      actions={
        <button
          className="btn btn-sm btn-primary"
          disabled={query.data == null || busy}
          onClick={onCreate}
          title="Create a local review on this commit range"
        >
          {busy ? "Creating…" : "Create review"}
        </button>
      }
    />
  );
}
