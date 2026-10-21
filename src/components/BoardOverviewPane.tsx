import { useMemo } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api } from "../lib/api";
import { buildDigest, digestHeadline, isOpenCard, type Digest, type DigestItem } from "../lib/boardDigest";
import { useAiDisabledReason, useSettingsStore } from "../lib/settings";
import { timeAgo } from "../lib/timeAgo";
import { useUIStore } from "../store";
import type { BoardCard } from "../lib/types";
import { Icon } from "./icons";

const REF_RE = /(?<![\w/[#-])((?:[\w.-]+\/)?[\w.-]+)#(\d+)\b/g;
const CARD_HREF = "#card=";

// Models abbreviate repo names (cardiologs-front → front), so the number is the
// primary key and the repo ref only disambiguates.
function findCard(cards: BoardCard[], repoRef: string, number: number): BoardCard | undefined {
  const candidates = cards.filter((c) => c.number === number);
  if (candidates.length <= 1) return candidates[0];
  const ref = repoRef.toLowerCase();
  const short = (c: BoardCard) => c.repo.toLowerCase().split("/").pop() ?? "";
  return (
    candidates.find((c) => c.repo.toLowerCase() === ref || short(c) === ref) ??
    candidates.find((c) => short(c).endsWith(`-${ref}`) || short(c).includes(ref))
  );
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
  const aiOff = useAiDisabledReason();
  const generate = useMutation({
    mutationFn: () => api.boardOverviewGenerate(overviewModel),
    onSuccess: (data) => qc.setQueryData(["board-overview"], data),
  });

  const overview = query.data;
  const drift = useMemo(() => {
    if (!overview) return null;
    const open = new Set(cards.filter(isOpenCard).map((c) => c.id));
    const before = new Set(overview.card_ids);
    const added = [...open].filter((id) => !before.has(id)).length;
    const removed = [...before].filter((id) => !open.has(id)).length;
    return added || removed ? { added, removed } : null;
  }, [overview, cards]);
  const digest = useMemo(() => buildDigest(cards, new Date()), [cards]);
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
        <strong>Overview</strong>
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
          disabled={generate.isPending || aiOff != null}
          title={aiOff ?? "Regenerate the overview from the current board (local data only)"}
        >
          {generate.isPending ? (
            <>
              <span className="spinner" /> Analyzing…
            </>
          ) : (
            <>
              <Icon name="refresh" size={12} /> Refresh insights
            </>
          )}
        </button>
      </div>
      <div className="board-overview-body">
        <div className="board-overview-insights">
        <div className="board-digest-headline">AI insights</div>
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
              No insights yet. Click <em>Refresh insights</em> for themes, sequencing and risks
              the digest below can't see.
            </span>
          )
        )}
        </div>
        <DigestView digest={digest} onOpen={openCard} />
      </div>
    </section>
  );
}

function DigestSection({
  title,
  items,
  onOpen,
}: {
  title: string;
  items: DigestItem[];
  onOpen: (id: string) => void;
}) {
  if (items.length === 0) return null;
  return (
    <>
      <div className="board-digest-title">{title}</div>
      <ul className="board-digest-list">
        {items.map((i, n) => (
          <li key={`${i.cardId}-${n}`} className="board-digest-row">
            <button className="board-overview-ref" onClick={() => onOpen(i.cardId)}>
              {i.ref}
            </button>
            <span>{i.reason}</span>
            <span className="board-digest-title-text">{i.title}</span>
          </li>
        ))}
      </ul>
    </>
  );
}

function DigestView({ digest, onOpen }: { digest: Digest; onOpen: (id: string) => void }) {
  if (digest.open === 0) return <span className="muted">Nothing open on the board.</span>;
  return (
    <div>
      <div className="board-digest-headline">{digestHeadline(digest)}</div>
      <DigestSection title="Do next" items={digest.doNext} onOpen={onOpen} />
      <DigestSection title="Blocked" items={digest.blocked} onOpen={onOpen} />
      <DigestSection title="Waiting on others" items={digest.waiting} onOpen={onOpen} />
      <DigestSection title="Stale" items={digest.stale} onOpen={onOpen} />
      <div className="board-digest-title">By project</div>
      <ul className="board-digest-list">
        {digest.byProject.map((p) => (
          <li key={p.repo} className="board-digest-row">
            <strong>{p.repo.split("/").pop()}</strong>
            <span className="muted">
              {[
                p.reviews && `${p.reviews} to review`,
                p.submitted && `${p.submitted} submitted`,
                p.in_progress && `${p.in_progress} in progress`,
                p.backlog && `${p.backlog} backlog`,
              ]
                .filter(Boolean)
                .join(" · ") || "no lane"}
            </span>
          </li>
        ))}
      </ul>
      <DigestSection title="Hygiene" items={digest.hygiene} onOpen={onOpen} />
    </div>
  );
}
