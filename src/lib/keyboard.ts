export interface Binding {
  keys: string[]; // display form, e.g. ["]"], ["n"], ["?"]
  description: string;
}

/** Single source of truth: drives BOTH the dispatch table and the help overlay,
 *  so they cannot drift. Order = display order. */
export const BINDINGS: Binding[] = [
  { keys: ["]", "["], description: "Next / previous file" },
  { keys: ["n", "p"], description: "Next / previous comment thread" },
  { keys: ["j", "k"], description: "Move line cursor down / up (active file)" },
  { keys: ["c"], description: "Comment on the focused line" },
  { keys: ["?"], description: "Toggle this help" },
  { keys: ["Esc"], description: "Close help / composer / line cursor" },
];

/** True when keyboard shortcuts must not fire because the user is typing in a
 *  field (input/textarea/select/contenteditable). */
export function isEditableTarget(target: EventTarget | null): boolean {
  return (
    target instanceof Element &&
    target.closest("input, textarea, select, [contenteditable]") != null
  );
}

/**
 * Index of the next/prev thread given element tops (scroll-space, ascending
 * document order) and the current scroll offset. `eps` absorbs "already there"
 * (a thread within `eps` of the current offset is treated as the current one).
 * Returns null when there is nothing further in that direction.
 *
 * dir=1: smallest i with tops[i] > current + eps.
 * dir=-1: largest i with tops[i] < current - eps.
 */
export function pickThread(
  tops: number[],
  current: number,
  dir: 1 | -1,
  eps = 8,
): number | null {
  if (dir === 1) {
    for (let i = 0; i < tops.length; i++) {
      if (tops[i] > current + eps) return i;
    }
    return null;
  }
  for (let i = tops.length - 1; i >= 0; i--) {
    if (tops[i] < current - eps) return i;
  }
  return null;
}

/**
 * Next cursor key for j/k. `current==null` starts at keys[0]; an unknown
 * `current` (e.g. the diff reparsed) restarts at keys[0]; otherwise the index
 * moves by `delta`, clamped at both ends (no wrap). Empty keys -> null.
 */
export function moveCursorKey(
  keys: string[],
  current: string | null,
  delta: 1 | -1,
): string | null {
  if (keys.length === 0) return null;
  if (current == null) return keys[0];
  const i = keys.indexOf(current);
  if (i === -1) return keys[0];
  const next = Math.min(keys.length - 1, Math.max(0, i + delta));
  return keys[next];
}

/** Handle each FileReview registers (index = diff file index). */
export interface FileKbHandle {
  /** j/k; no-op when the file is viewed/collapsed or has no lines. */
  moveCursor: (delta: 1 | -1) => void;
  /** c; no-op without a cursor or when readOnly. */
  openComposer: () => void;
  /** Escape; true if it consumed (closed selection or cleared cursor). */
  clearCursor: () => boolean;
}

/** Handle FileJumpList publishes (refreshed every render). */
export interface JumpListHandle {
  activeIndex: number;
  fileCount: number;
  jumpTo: (index: number) => void;
}
