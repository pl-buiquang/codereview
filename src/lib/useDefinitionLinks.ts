import { useEffect, useRef, type RefObject } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { api } from "./api";
import {
  identifierUnderPointer,
  showDefinitionLink,
  type IdentifierUnderPointer,
} from "./definition";
import type { Side } from "./types";

/** What a Cmd/Ctrl-click asked for; `x`/`y` is the click position in the viewport. */
export interface DefinitionLookup {
  symbol: string;
  side: Side;
  /** Path of the file the word was clicked in. */
  path: string;
  /** Where it was clicked: the review's diff or the head-file pane. */
  from: "diff" | "file";
  x: number;
  y: number;
}

export const definitionQueryKey = (reviewId: number, pin: string, side: Side, path: string, symbol: string) =>
  ["definition", reviewId, pin, side, path, symbol] as const;

/** The pointer must rest on a word this long before we ask the backend about it. */
const SETTLE_MS = 120;
/** Results (including "none") are good for this long; the pin is part of the key. */
export const DEFINITION_STALE_MS = 5 * 60_000;

interface Probe {
  symbol: string;
  side: Side;
  path: string;
  hit: IdentifierUnderPointer;
}

/** The code word under a viewport point, or null off the code cells (diff or file pane). */
function probeAt(x: number, y: number): Probe | null {
  const cell = document.elementFromPoint(x, y)?.closest<HTMLElement>("td.diff-code");
  const path = cell?.closest<HTMLElement>("[data-file-path]")?.dataset.filePath;
  if (!cell || !path) return null;
  const hit = identifierUnderPointer({ clientX: x, clientY: y });
  if (!hit || !cell.contains(hit.node)) return null;
  const side: Side = cell.classList.contains("diff-code-delete") ? "LEFT" : "RIGHT";
  return { symbol: hit.word, side, path, hit };
}

/**
 * Cmd/Ctrl-hover and -click on identifiers inside `rootRef` (the diff area).
 *
 * One delegated listener set for the whole diff, so it costs the same for a huge diff and never
 * touches the (memoized) diff rows. While Cmd/Ctrl is held and the pointer rests on a word, the
 * backend is asked (cheap, cached) whether it has a declaration; if so the word is underlined
 * with a CSS custom highlight and the cursor becomes a pointer. A click reports a
 * [`DefinitionLookup`] to `onLookup`.
 */
export function useDefinitionLinks({
  rootRef,
  from,
  enabled,
  reviewId,
  pin,
  diffPaths,
  onLookup,
}: {
  rootRef: RefObject<HTMLElement | null>;
  from: "diff" | "file";
  /** False while the review's tab is hidden: its subtree stays mounted. */
  enabled: boolean;
  reviewId: number;
  /** `base:head` commit ids; results are only valid for this pair. */
  pin: string;
  diffPaths: string[];
  onLookup: (lookup: DefinitionLookup) => void;
}) {
  const queryClient = useQueryClient();
  const live = useRef({ diffPaths, onLookup });
  live.current = { diffPaths, onLookup };

  useEffect(() => {
    const root = rootRef.current;
    if (!enabled || !root) return;
    let held = false;
    let pointer: { x: number; y: number } | null = null;
    let current: IdentifierUnderPointer | null = null;
    let settle: number | undefined;
    let raf = 0;
    let token = 0;

    const clear = () => {
      window.clearTimeout(settle);
      token += 1;
      current = null;
      showDefinitionLink(null);
      root.classList.remove("def-link-hover");
    };

    const evaluate = () => {
      raf = 0;
      if (!held || !pointer) return;
      const probe = probeAt(pointer.x, pointer.y);
      if (probe && current && probe.hit.node === current.node && probe.hit.start === current.start) return;
      clear();
      if (!probe) return;
      current = probe.hit;
      const mine = token;
      settle = window.setTimeout(async () => {
        try {
          const hits = await queryClient.fetchQuery({
            queryKey: definitionQueryKey(reviewId, pin, probe.side, probe.path, probe.symbol),
            queryFn: () =>
              api.findDefinition(reviewId, probe.symbol, probe.path, probe.side, live.current.diffPaths, true),
            staleTime: DEFINITION_STALE_MS,
          });
          if (mine !== token || !hits.length) return;
          showDefinitionLink(probe.hit);
          root.classList.add("def-link-hover");
        } catch {
          // superseded ("cancelled") or failed: no link, the click path reports real errors
        }
      }, SETTLE_MS);
    };
    const schedule = () => {
      if (!raf) raf = requestAnimationFrame(evaluate);
    };

    const onMove = (e: MouseEvent) => {
      pointer = { x: e.clientX, y: e.clientY };
      if (held) schedule();
    };
    const setHeld = (next: boolean) => {
      if (next === held) return;
      held = next;
      if (held) schedule();
      else clear();
    };
    const onKey = (e: KeyboardEvent) => setHeld(e.metaKey || e.ctrlKey);
    const onBlur = () => setHeld(false);
    const onLeave = () => {
      pointer = null;
      clear();
    };
    const onMouseDown = (e: MouseEvent) => {
      // no text selection / word-select from a Cmd-click on a word
      if ((e.metaKey || e.ctrlKey) && e.button === 0 && probeAt(e.clientX, e.clientY)) e.preventDefault();
    };
    const onClick = (e: MouseEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.button !== 0) return;
      const probe = probeAt(e.clientX, e.clientY);
      if (!probe) return;
      e.preventDefault();
      e.stopPropagation();
      window.getSelection()?.removeAllRanges();
      clear();
      live.current.onLookup({
        symbol: probe.symbol,
        side: probe.side,
        path: probe.path,
        from,
        x: e.clientX,
        y: e.clientY,
      });
    };

    root.addEventListener("mousemove", onMove);
    root.addEventListener("mouseleave", onLeave);
    root.addEventListener("mousedown", onMouseDown, true);
    root.addEventListener("click", onClick, true);
    window.addEventListener("keydown", onKey);
    window.addEventListener("keyup", onKey);
    window.addEventListener("blur", onBlur);
    return () => {
      root.removeEventListener("mousemove", onMove);
      root.removeEventListener("mouseleave", onLeave);
      root.removeEventListener("mousedown", onMouseDown, true);
      root.removeEventListener("click", onClick, true);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keyup", onKey);
      window.removeEventListener("blur", onBlur);
      cancelAnimationFrame(raf);
      clear();
    };
  }, [rootRef, from, enabled, reviewId, pin, queryClient]);
}
