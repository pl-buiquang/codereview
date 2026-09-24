import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { useCommandPaletteStore } from "../lib/commandPalette";
import { searchPalette, type PaletteResult } from "../lib/commandPaletteSearch";
import { parseRepoBasePaths, useSettingsStore } from "../lib/settings";
import { timeAgo } from "../lib/timeAgo";
import { toast } from "../lib/toast";
import { useUIStore } from "../store";
import { Icon } from "./icons";

export function CommandPalette() {
  const open = useCommandPaletteStore((s) => s.open);
  if (!open) return null;
  return <CommandPaletteInner />;
}

function CommandPaletteInner() {
  const hide = useCommandPaletteStore((s) => s.hide);
  const openReview = useUIStore((s) => s.openReview);
  const queryClient = useQueryClient();
  const repoBasePaths = useSettingsStore((s) => s.repoBasePaths);

  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const [pending, setPending] = useState(false);

  const inputRef = useRef<HTMLInputElement>(null);
  const activeItemRef = useRef<HTMLButtonElement>(null);
  const previousFocusRef = useRef<Element | null>(null);

  const reviewsQuery = useQuery({
    queryKey: ["reviews", null],
    queryFn: () => api.listReviews(null),
  });
  const reposQuery = useQuery({
    queryKey: ["repositories"],
    queryFn: api.listRepositories,
  });

  const results: PaletteResult[] = searchPalette(
    query,
    reviewsQuery.data ?? [],
    reposQuery.data ?? [],
  );

  // Save focus target on mount, restore on unmount
  useEffect(() => {
    previousFocusRef.current = document.activeElement;
    inputRef.current?.focus();
    return () => {
      (previousFocusRef.current as HTMLElement | null)?.focus();
    };
  }, []);

  // Scroll active item into view
  useEffect(() => {
    activeItemRef.current?.scrollIntoView({ block: "nearest" });
  }, [activeIndex]);

  // Reset active index when results change
  useEffect(() => {
    setActiveIndex(0);
  }, [query]);

  async function execute(result: PaletteResult) {
    if (result.kind === "review") {
      hide();
      openReview(result.reviewId, result.repoId);
      return;
    }

    // create-pr
    setPending(true);
    hide();
    try {
      const review = await api.createReviewForPr(result.owner, result.name, result.prNumber, parseRepoBasePaths(repoBasePaths));
      await queryClient.invalidateQueries({ queryKey: ["reviews"] });
      openReview(review.id);
    } catch (e) {
      toast.error(`Failed to open PR #${result.prNumber}: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setPending(false);
    }
  }

  function onKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      hide();
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActiveIndex((i) => Math.min(i + 1, results.length - 1));
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      setActiveIndex((i) => Math.max(i - 1, 0));
      return;
    }
    if (e.key === "Enter" && results.length > 0) {
      e.preventDefault();
      execute(results[activeIndex]);
      return;
    }
  }

  const reviewItems = results.filter((r): r is Extract<PaletteResult, { kind: "review" }> => r.kind === "review");
  const createItems = results.filter((r): r is Extract<PaletteResult, { kind: "create-pr" }> => r.kind === "create-pr");

  const reviewOffset = 0;
  const createOffset = reviewItems.length;

  return (
    <div className="modal-backdrop command-palette-backdrop" onClick={hide}>
      <div className="modal command-palette" onClick={(e) => e.stopPropagation()} onKeyDown={onKeyDown}>
        <div className="command-palette-input-row">
          <Icon name="search" size={14} style={{ color: "var(--text-3)" }} />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search reviews, type 'repo 123', or paste a GitHub PR URL…"
            disabled={pending}
          />
        </div>

        {results.length > 0 && (
          <div className="command-palette-results">
            {reviewItems.length > 0 && (
              <>
                <div className="command-palette-section">Reviews</div>
                {reviewItems.map((r, i) => (
                  <button
                    key={r.reviewId}
                    ref={reviewOffset + i === activeIndex ? activeItemRef : undefined}
                    className={`command-palette-item${reviewOffset + i === activeIndex ? " active" : ""}`}
                    onClick={() => execute(r)}
                    onMouseEnter={() => setActiveIndex(reviewOffset + i)}
                  >
                    <Icon name="review" size={13} style={{ flex: "none", color: "var(--text-2)" }} />
                    <span className="title">{r.title}</span>
                    <span className="meta">
                      {r.prNumber != null
                        ? `${r.repoLabel.split("/")[1] ?? r.repoLabel} #${r.prNumber}`
                        : r.repoLabel}
                      {r.commentCount > 0 ? ` · ${r.commentCount}` : ""}
                      {" · "}
                      {timeAgo(r.updatedAt)}
                    </span>
                  </button>
                ))}
              </>
            )}

            {createItems.length > 0 && (
              <>
                <div className="command-palette-section">Open new</div>
                {createItems.map((r, i) => (
                  <button
                    key={`${r.owner}/${r.name}#${r.prNumber}`}
                    ref={createOffset + i === activeIndex ? activeItemRef : undefined}
                    className={`command-palette-item action${createOffset + i === activeIndex ? " active" : ""}`}
                    onClick={() => execute(r)}
                    onMouseEnter={() => setActiveIndex(createOffset + i)}
                  >
                    <Icon name="plus" size={13} style={{ flex: "none", color: "var(--accent)" }} />
                    <span className="title">
                      Review PR #{r.prNumber} on {r.repoLabel}
                    </span>
                  </button>
                ))}
              </>
            )}
          </div>
        )}

        {results.length === 0 && query.trim() && (
          <div className="command-palette-empty">No results</div>
        )}

        <div className="command-palette-footer">
          <span><kbd>↑↓</kbd> navigate</span>
          <span><kbd>↵</kbd> open</span>
          <span><kbd>esc</kbd> close</span>
        </div>
      </div>
    </div>
  );
}
