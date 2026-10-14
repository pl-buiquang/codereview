import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { toast } from "../lib/toast";

/** "+ Track" modal: paste a GitHub PR URL to add it to the board. Refuses
 *  issues and PRs already tracked, surfacing the error inline instead of
 *  closing — so the user can fix the input and retry. */
export function TrackPrDialog({ onClose }: { onClose: () => void }) {
  const queryClient = useQueryClient();
  const [reference, setReference] = useState("");

  const track = useMutation({
    mutationFn: (ref: string) => api.boardTrackPr(ref),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["board"] });
      toast.success("PR tracked");
      onClose();
    },
  });

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    const v = reference.trim();
    if (!v) return;
    track.mutate(v);
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") onClose();
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal track-pr-modal" onClick={(e) => e.stopPropagation()} onKeyDown={onKeyDown} role="dialog" aria-modal="true">
        <div className="modal-header">
          <h3>Track a PR</h3>
        </div>
        <form onSubmit={submit}>
          <input
            className="input"
            autoFocus
            placeholder="https://github.com/owner/repo/pull/123"
            value={reference}
            onChange={(e) => setReference(e.target.value)}
          />
          {track.isError && <p className="form-warning">{String(track.error)}</p>}
          <div className="modal-actions">
            <button type="button" onClick={onClose}>
              Cancel
            </button>
            <button type="submit" className="btn-primary" disabled={!reference.trim() || track.isPending}>
              {track.isPending ? "Tracking…" : "Track"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
