import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { AddMetaForm, MetaRow } from "./BoardCard";
import { Icon } from "./icons";
import type { BoardMetaKind } from "../lib/types";

/** The review's left-pane "Context" tab: the linked board card's metadata
 *  (plan/spec files, links, notes, Claude sessions), including items
 *  inherited from a linked issue. Mirrors the same add/delete UI as the
 *  board card itself, since both edit the same `board_meta` rows. */
export function ContextPane({ owner, name, number }: { owner: string; name: string; number: number }) {
  const queryClient = useQueryClient();
  const [adding, setAdding] = useState(false);

  const cardQueryKey = ["board-card-for-pr", owner, name, number];
  const cardQuery = useQuery({
    queryKey: cardQueryKey,
    queryFn: () => api.boardCardForPr(owner, name, number),
  });
  const tagsQuery = useQuery({ queryKey: ["board-meta-tags"], queryFn: api.boardMetaTags });

  const onErr = (e: unknown) => toast.error(String(e));
  const invalidate = () => {
    queryClient.invalidateQueries({ queryKey: cardQueryKey });
    queryClient.invalidateQueries({ queryKey: ["board"] });
  };

  const track = useMutation({
    mutationFn: () => api.boardTrack(`${owner}/${name}#${number}`),
    onSuccess: invalidate,
    onError: onErr,
  });
  const metaAdd = useMutation({
    mutationFn: (args: { cardId: string; kind: BoardMetaKind; value: string; tag: string | null; label: string | null }) =>
      api.boardMetaAdd(args),
    onSuccess: () => {
      invalidate();
      queryClient.invalidateQueries({ queryKey: ["board-meta-tags"] });
    },
    onError: onErr,
  });
  const metaDelete = useMutation({
    mutationFn: (id: number) => api.boardMetaDelete(id),
    onSuccess: invalidate,
    onError: onErr,
  });

  if (cardQuery.isLoading) return <div className="context-pane muted">Loading context…</div>;
  if (cardQuery.isError)
    return <div className="context-pane muted">Could not load board card: {String(cardQuery.error)}</div>;

  const card = cardQuery.data;

  if (!card) {
    return (
      <div className="context-pane context-pane-empty">
        <p className="muted">This PR isn't tracked on the board.</p>
        <button className="btn btn-sm" disabled={track.isPending} onClick={() => track.mutate()}>
          {track.isPending ? "Tracking…" : "Track on board"}
        </button>
      </div>
    );
  }

  const allMeta = [...card.meta, ...card.inherited_meta];
  const inheritedIds = new Set(card.inherited_meta.map((m) => m.id));

  return (
    <div className="context-pane">
      {card.discarded_at && (
        <div className="context-pane-note faint">
          Discarded from the board.{" "}
          <button disabled={track.isPending} onClick={() => track.mutate()}>
            Re-track
          </button>
        </div>
      )}
      {card.archived_at && !card.discarded_at && (
        <div className="context-pane-note faint">Archived from the board.</div>
      )}

      {allMeta.length === 0 && !adding && <p className="muted">No metadata yet.</p>}

      {allMeta.length > 0 && (
        <div className="board-meta-list">
          {allMeta.map((m) => (
            <MetaRow key={m.id} meta={m} inherited={inheritedIds.has(m.id)} onDelete={() => metaDelete.mutate(m.id)} />
          ))}
        </div>
      )}

      {adding ? (
        <AddMetaForm
          tags={tagsQuery.data ?? []}
          onAdd={(args) => metaAdd.mutate({ cardId: card.id, ...args })}
          onClose={() => setAdding(false)}
        />
      ) : (
        <button className="btn-icon context-pane-add-meta" title="Add metadata" onClick={() => setAdding(true)}>
          <Icon name="plus" size={13} /> Add
        </button>
      )}
    </div>
  );
}
