import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { useUIStore } from "../store";
import type { InboxItem } from "../lib/types";
import { InboxItemRow } from "./InboxItemRow";

export function ArchiveView() {
  const queryClient = useQueryClient();
  const openReview = useUIStore((s) => s.openReview);
  const [search, setSearch] = useState("");

  const archiveQuery = useQuery({
    queryKey: ["archive", search.trim()],
    queryFn: () => api.listArchive(search.trim() || null),
  });

  const invalidate = () => {
    queryClient.invalidateQueries({ queryKey: ["archive"] });
    queryClient.invalidateQueries({ queryKey: ["inbox"] });
  };

  const retrack = useMutation({
    mutationFn: api.retrackItem,
    onSuccess: invalidate,
    onError: (e) => toast.error(String(e)),
  });

  const openPr = useMutation({
    mutationFn: (item: InboxItem) => {
      const [owner, name] = item.repo.split("/");
      return api.openPrReview(item.id, owner, name, item.number);
    },
    onSuccess: (review) => {
      invalidate();
      openReview(review.id);
    },
    onError: (e) => toast.error(`Could not open review:\n${String(e)}`),
  });

  const items = archiveQuery.data ?? [];

  return (
    <section className="main-panel inbox-panel">
      <header className="inbox-header">
        <h2 className="inbox-h">Archive</h2>
        <input
          className="archive-search"
          placeholder="Search archived by title, repo, or author…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
      </header>

      <div className="inbox-list">
        {archiveQuery.isLoading && <p className="muted">Loading…</p>}
        {!archiveQuery.isLoading && items.length === 0 && (
          <p className="muted">{search.trim() ? "No matches." : "Nothing archived."}</p>
        )}
        {items.map((item) => (
          <InboxItemRow
            key={item.id}
            item={item}
            variant="archive"
            busy={retrack.isPending || openPr.isPending}
            onEngage={() => {}}
            onUnengage={() => {}}
            onUntrack={() => {}}
            onRetrack={() => retrack.mutate(item.id)}
            onOpenReview={() => openPr.mutate(item)}
          />
        ))}
      </div>
    </section>
  );
}
