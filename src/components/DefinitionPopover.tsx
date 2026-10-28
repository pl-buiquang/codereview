import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import {
  DEFINITION_STALE_MS,
  definitionQueryKey,
  type DefinitionLookup,
} from "../lib/useDefinitionLinks";
import type { Side } from "../lib/types";

const WIDTH = 460;

/**
 * Popover for a Cmd/Ctrl-clicked identifier in the diff. It opens at once with a spinner; the
 * result usually is already cached by the hover check. A single declaration that is part of the
 * diff is jumped to directly (the popover closes itself). Otherwise each declaration is listed
 * with a few lines of source and a way to reach it.
 *
 * Rendered into `document.body`: the diff files use `content-visibility`/`overflow: clip`, which
 * would make them the containing block of a `position: fixed` popover and clip it.
 */
export function DefinitionPopover({
  reviewId,
  pin,
  lookup,
  diffPaths,
  inDiff,
  onJump,
  onOpenFile,
  onClose,
}: {
  reviewId: number;
  pin: string;
  lookup: DefinitionLookup;
  diffPaths: string[];
  /** True when `side:line` of `path` is a line the diff view shows. */
  inDiff: (path: string, side: Side, line: number) => boolean;
  onJump: (path: string, side: Side, line: number) => void;
  /** Open the head-side file pane at `line`. */
  onOpenFile: (path: string, line: number) => void;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const { symbol, side, path, x, y } = lookup;
  const query = useQuery({
    queryKey: definitionQueryKey(reviewId, pin, side, path, symbol),
    queryFn: () => api.findDefinition(reviewId, symbol, path, side, diffPaths, false),
    staleTime: DEFINITION_STALE_MS,
    // a hover lookup we shared a request with may have been superseded; ask again
    retry: (count, error) => count < 3 && String(error).includes("cancelled"),
  });

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousedown", onDown);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onDown);
    };
  }, [onClose]);

  const hits = query.data;
  const only = hits?.length === 1 ? hits[0] : null;
  const jumpOnly = only != null && inDiff(only.path, side, only.line);
  useEffect(() => {
    if (!only || !jumpOnly) return;
    onJump(only.path, side, only.line);
    onClose();
  }, [only, jumpOnly, side, onJump, onClose]);

  const left = Math.max(8, Math.min(x, window.innerWidth - WIDTH - 8));
  const top = Math.min(y + 14, window.innerHeight - 120);

  return createPortal(
    <div
      ref={ref}
      className="definition-popover"
      style={{ left, top, width: WIDTH, maxHeight: window.innerHeight - top - 8 }}
      role="dialog"
      aria-label={`Definition of ${symbol}`}
    >
      <div className="definition-popover-head">
        <code>{symbol}</code>
        {query.isPending && <span className="spinner" />}
        <span className="muted">
          {query.isPending
            ? "searching…"
            : query.isError
              ? "search failed"
              : query.data.length === 0
                ? "no definition found"
                : `${query.data.length} definition${query.data.length > 1 ? "s" : ""}`}
        </span>
      </div>
      {query.isError && <p className="muted">{String(query.error)}</p>}
      {!jumpOnly &&
        hits?.map((hit) => (
          <div key={`${hit.path}:${hit.line}`} className="definition-hit">
            <div className="definition-hit-head">
              <span className="file-path" title={hit.path}>
                {hit.path}:{hit.line}
              </span>
              <span className="muted">{hit.kind}</span>
              {inDiff(hit.path, side, hit.line) && (
                <button
                  className="link-button"
                  onClick={() => {
                    onJump(hit.path, side, hit.line);
                    onClose();
                  }}
                >
                  Go to line
                </button>
              )}
              {/* the file pane shows the head side only */}
              {side === "RIGHT" && (
                <button
                  className="link-button"
                  onClick={() => {
                    onOpenFile(hit.path, hit.line);
                    onClose();
                  }}
                >
                  Open file
                </button>
              )}
            </div>
            <pre className="definition-snippet">{hit.snippet}</pre>
          </div>
        ))}
    </div>,
    document.body,
  );
}
