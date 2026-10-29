import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { timeAgo } from "../lib/timeAgo";
import { statusLabel } from "../lib/status";
import { tagLabel, type CommentTag } from "../lib/commentTag";

const TYPE_ORDER = ["mine", "ai", "ai:explain", "ai:review", "ai:codex", "local"] as const;

function typeLabel(t: string): string {
  return t === "mine" ? "Mine" : tagLabel(t as CommentTag);
}

/** "Import comments" modal: pick an earlier review of this PR and the comment
 *  types to copy into the current draft. Published comments are never offered;
 *  the backend skips duplicates and re-anchors what it copied. */
export function ImportCommentsDialog({
  reviewId,
  targetId,
  onClose,
}: {
  reviewId: number;
  targetId: number;
  onClose: () => void;
}) {
  const queryClient = useQueryClient();
  const reviewsQuery = useQuery({
    queryKey: ["target-reviews", targetId],
    queryFn: () => api.listTargetReviews(targetId),
  });
  const sources = (reviewsQuery.data ?? []).filter((r) => r.review.id !== reviewId);
  const [sourceId, setSourceId] = useState<number | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const activeSource = sourceId ?? sources[0]?.review.id ?? null;

  const sourceQuery = useQuery({
    queryKey: ["review", activeSource],
    queryFn: () => api.getReview(activeSource!),
    enabled: activeSource != null,
  });

  const counts = useMemo(() => {
    const m = new Map<string, number>();
    for (const c of sourceQuery.data?.comments ?? []) {
      if (c.parent_id != null || c.github_comment_id != null || c.github_thread_root_id != null) continue;
      const k = c.tag ?? "mine";
      m.set(k, (m.get(k) ?? 0) + 1);
    }
    return m;
  }, [sourceQuery.data]);

  // Every available type starts checked whenever the source changes.
  useEffect(() => {
    setSelected(new Set(counts.keys()));
  }, [counts]);

  const types = TYPE_ORDER.filter((t) => counts.has(t));
  const total = types.filter((t) => selected.has(t)).reduce((n, t) => n + (counts.get(t) ?? 0), 0);

  const doImport = useMutation({
    mutationFn: () => api.importComments(reviewId, activeSource!, [...selected]),
    onSuccess: (r) => {
      queryClient.invalidateQueries({ queryKey: ["review", reviewId] });
      queryClient.invalidateQueries({ queryKey: ["reviews"] });
      queryClient.invalidateQueries({ queryKey: ["target-reviews", targetId] });
      const parts = [`Imported ${r.imported} comment${r.imported === 1 ? "" : "s"}`];
      if (r.skipped) parts.push(`${r.skipped} skipped (duplicates)`);
      if (r.reanchor) {
        if (r.reanchor.reanchored) parts.push(`${r.reanchor.reanchored} moved`);
        if (r.reanchor.lost) parts.push(`${r.reanchor.lost} outdated`);
      }
      toast.success(parts.join(", "));
      if (r.reanchorError) toast.error(`Re-anchor failed:\n${r.reanchorError}`);
      onClose();
    },
    onError: (e) => toast.error(`Import failed:\n${String(e)}`),
  });

  const toggle = (t: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (!next.delete(t)) next.add(t);
      return next;
    });

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal"
        role="dialog"
        aria-modal="true"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.key === "Escape" && onClose()}
      >
        <div className="modal-header">
          <h3>Import comments</h3>
          <button className="btn-icon" onClick={onClose}>
            ✕
          </button>
        </div>
        <p className="muted">Copy comments from an earlier review on this PR. Published comments aren't included.</p>
        <label className="muted" htmlFor="import-source">
          From review
        </label>
        <select
          id="import-source"
          className="input"
          autoFocus
          value={activeSource ?? ""}
          onChange={(e) => setSourceId(Number(e.target.value))}
        >
          {sources.map((r) => (
            <option key={r.review.id} value={r.review.id}>
              {statusLabel(r.review.status)} · {timeAgo(r.review.updated_at)} · {r.comment_count} comment
              {r.comment_count === 1 ? "" : "s"}
            </option>
          ))}
        </select>
        <div className="import-types">
          {sourceQuery.isLoading && <span className="faint">Loading…</span>}
          {sourceQuery.data && types.length === 0 && <span className="faint">No importable comments in this review.</span>}
          {types.map((t) => (
            <label key={t} className="import-type">
              <input type="checkbox" checked={selected.has(t)} onChange={() => toggle(t)} />
              <span>{typeLabel(t)}</span>
              <span className="faint">{counts.get(t)}</span>
            </label>
          ))}
        </div>
        <div className="modal-actions">
          <button onClick={onClose}>Cancel</button>
          <button
            className="btn-primary"
            disabled={total === 0 || doImport.isPending || activeSource == null}
            onClick={() => doImport.mutate()}
          >
            {doImport.isPending ? "Importing…" : `Import ${total} comment${total === 1 ? "" : "s"}`}
          </button>
        </div>
      </div>
    </div>
  );
}
