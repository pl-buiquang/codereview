import type { FileData } from "react-diff-view";
import { indexFile } from "./diff";

export interface DiffMatch {
  fileIndex: number;
  changeKey: string;
}

/** Just the field of `ChangeMeta` search needs, so callers can pass a precomputed
 *  `indexFile(...).metaByKey` without coupling to the full shape. */
type MetaLike = Map<string, { lineText: string }>;

/**
 * Diff-line matches for `query` (case-insensitive substring) across every file's
 * change metadata, in file-then-document order. `metaByFile[i]` is file i's
 * `indexFile(...).metaByKey`. Blank query → no matches. The UI builds the
 * per-file index ONCE (it's stable across keystrokes) and re-filters here.
 */
export function filterDiffMatches(metaByFile: MetaLike[], query: string): DiffMatch[] {
  const q = query.trim().toLowerCase();
  if (!q) return [];
  const out: DiffMatch[] = [];
  metaByFile.forEach((meta, fileIndex) => {
    for (const [changeKey, m] of meta) {
      if (m.lineText.toLowerCase().includes(q)) out.push({ fileIndex, changeKey });
    }
  });
  return out;
}

/** Convenience wrapper that indexes each file then filters — used in tests. */
export function findDiffMatches(files: FileData[], query: string): DiffMatch[] {
  return filterDiffMatches(
    files.map((f) => indexFile(f).metaByKey),
    query,
  );
}

/** Wrap `i` into `[0, len)` for next/prev cycling; `len <= 0` → 0. */
export function wrapIndex(i: number, len: number): number {
  if (len <= 0) return 0;
  return ((i % len) + len) % len;
}
