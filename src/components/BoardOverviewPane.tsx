import { useMemo } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api } from "../lib/api";
import { useSettingsStore } from "../lib/settings";
import { timeAgo } from "../lib/timeAgo";
import { useUIStore } from "../store";
import type { BoardCard } from "../lib/types";
import { Icon } from "./icons";

const REF_RE = /(?<![\w/[#-])((?:[\w.-]+\/)?[\w.-]+)#(\d+)\b/g;
const CARD_HREF = "#card=";

function findCard(cards: BoardCard[], repoRef: string, number: number): BoardCard | undefined {
  const ref = repoRef.toLowerCase();
  return cards.find((c) => {
    if (c.number !== number) return false;
    const repo = c.repo.toLowerCase();
    return repo === ref || repo.endsWith(`/${ref}`);
  });
}

/** Turns `repo#N` / `owner/repo#N` into markdown links to the matching board card. */
export function linkCardRefs(markdown: string, cards: BoardCard[]): string {
  return markdown.replace(REF_RE, (whole, repoRef: string, num: string) => {
    const card = findCard(cards, repoRef, Number(num));
    return card ? `[${whole}](${CARD_HREF}${encodeURIComponent(card.id)})` : whole;
  });
}

export function BoardOverviewPane({
  cards,
  visibleIds,
}: {
  cards: BoardCard[];
  visibleIds: Set<string>;
}) {
  const qc = useQueryClient();
  const overviewModel = useSettingsStore((s) => s.overviewModel);
  const showBoardCard = useUIStore((s) => s.showBoardCard);
  const query = useQuery({ queryKey: ["board-overview"], queryFn: api.boardOverviewGet });
  const generate = useMutation({
    mutationFn: () => api.boardOverviewGenerate(overviewModel),
    onSuccess: (data) => qc.setQueryData(["board-overview"], data),
  });

  const overview = query.data;
  const drift = useMemo(() => {
    if (!overview) return null;
    const open = new Set(
      cards
        .filter((c) => !c.archived_at && !c.discarded_at && (c.state == null || c.state === "open"))
        .map((c) => c.id),
    );
    const before = new Set(overview.card_ids);
    const added = [...open].filter((id) => !before.has(id)).length;
    const removed = [...before].filter((id) => !open.has(id)).length;
    return added || removed ? { added, removed } : null;
  }, [overview, cards]);
  const source = useMemo(
    () => (overview ? linkCardRefs(overview.markdown, cards) : ""),
    [overview, cards],
  );

  const openCard = (id: string) => {
    const card = cards.find((c) => c.id === id);
    if (!card) return;
    if (visibleIds.has(id)) showBoardCard(id);
    else api.openUrl(card.url);
  };

  const meta = overview
    ? [
        `generated ${timeAgo(overview.generated_at)}`,
        overview.model,
        `${overview.card_count} cards`,
        overview.cost_usd != null ? `$${overview.cost_usd.toFixed(3)}` : null,
      ]
        .filter(Boolean)
        .join(" · ")
    : null;

  return (
    <section className="board-overview">
      <div className="board-overview-head">
        <Icon name="bot" size={14} />
        <strong>AI Overview</strong>
        {meta && <span className="cr-sub">{meta}</span>}
        {drift && !generate.isPending && (
          <span className="board-overview-drift" title="Cards were added or removed since this overview">
            board changed: {drift.added > 0 && `+${drift.added}`}
            {drift.added > 0 && drift.removed > 0 && " "}
            {drift.removed > 0 && `−${drift.removed}`}
          </span>
        )}
        <span className="cr-spacer" />
        <button
          className="btn btn-sm"
          onClick={() => generate.mutate()}
          disabled={generate.isPending}
          title="Regenerate the overview from the current board (local data only)"
        >
          {generate.isPending ? (
            <>
              <span className="spinner" /> Analyzing…
            </>
          ) : (
            <>
              <Icon name="refresh" size={12} /> Refresh overview
            </>
          )}
        </button>
      </div>
      <div className="board-overview-body">
        {generate.error && <div className="error">{String(generate.error)}</div>}
        {query.isLoading ? (
          <span className="muted">Loading…</span>
        ) : overview ? (
          <div className="markdown-body">
            <ReactMarkdown
              remarkPlugins={[remarkGfm]}
              components={{
                a: ({ href, children }) => {
                  const cardId = href?.startsWith(CARD_HREF)
                    ? decodeURIComponent(href.slice(CARD_HREF.length))
                    : null;
                  return (
                    <a
                      href={href}
                      className={cardId ? "board-overview-ref" : undefined}
                      onClick={(e) => {
                        e.preventDefault();
                        if (cardId) openCard(cardId);
                        else if (href) api.openUrl(href);
                      }}
                    >
                      {children}
                    </a>
                  );
                },
              }}
            >
              {source}
            </ReactMarkdown>
          </div>
        ) : (
          !generate.isPending && (
            <span className="muted">
              No overview yet. Click <em>Refresh overview</em> to analyze the open cards on the
              board.
            </span>
          )
        )}
      </div>
    </section>
  );
}
