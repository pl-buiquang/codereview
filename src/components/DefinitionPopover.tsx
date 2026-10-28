import { useEffect, useRef } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import type { Side } from "../lib/types";

export interface DefinitionLookup {
  symbol: string;
  side: Side;
  /** Viewport position of the click. */
  x: number;
  y: number;
}

const WIDTH = 460;

/**
 * Popover for a Cmd/Ctrl-clicked identifier in the diff: the declarations `find_definition`
 * found, each with a few lines of source and a way to reach the file (scroll to it when it is
 * part of the diff, otherwise open it in the file pane).
 */
export function DefinitionPopover({
  reviewId,
  filePath,
  lookup,
  inDiff,
  onJumpInDiff,
  onOpenFile,
  onClose,
}: {
  reviewId: number;
  filePath: string;
  lookup: DefinitionLookup;
  inDiff: (path: string) => boolean;
  onJumpInDiff: (path: string) => void;
  onOpenFile: (path: string) => void;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const { symbol, side, x, y } = lookup;
  const query = useQuery({
    queryKey: ["definition", reviewId, side, filePath, symbol],
    queryFn: () => api.findDefinition(reviewId, symbol, filePath, side),
    staleTime: 60_000,
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

  const left = Math.max(8, Math.min(x, window.innerWidth - WIDTH - 8));
  const top = Math.min(y + 14, window.innerHeight - 120);

  return (
    <div
      ref={ref}
      className="definition-popover"
      style={{ left, top, width: WIDTH, maxHeight: window.innerHeight - top - 8 }}
      role="dialog"
      aria-label={`Definition of ${symbol}`}
    >
      <div className="definition-popover-head">
        <code>{symbol}</code>
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
      {query.data?.map((hit) => (
        <div key={`${hit.path}:${hit.line}`} className="definition-hit">
          <div className="definition-hit-head">
            <span className="file-path" title={hit.path}>
              {hit.path}:{hit.line}
            </span>
            {inDiff(hit.path) && (
              <button
                className="link-button"
                onClick={() => {
                  onJumpInDiff(hit.path);
                  onClose();
                }}
              >
                Go to in diff
              </button>
            )}
            <button
              className="link-button"
              onClick={() => {
                onOpenFile(hit.path);
                onClose();
              }}
            >
              Open file
            </button>
          </div>
          <pre className="definition-snippet">{hit.snippet}</pre>
        </div>
      ))}
    </div>
  );
}
