import {
  getChangeKey,
  markEdits,
  tokenize,
  type ChangeData,
  type FileData,
  type HunkData,
  type TokenizeOptions,
} from "react-diff-view";
import { refractor } from "refractor";
import type { Side } from "./types";

// react-diff-view@3 treats refractor.highlight()'s return value as an array of
// hast children, but refractor@5 wraps them in a root node ({type:'root',
// children:[...]}). Left as-is, tokenize() throws on the non-array and all
// highlighting silently falls back to plain text. Unwrap .children to restore
// the v4 shape tokenize() expects.
const refractorCompat = {
  ...refractor,
  highlight: (value: string, language: string) =>
    refractor.highlight(value, language).children,
} as unknown as typeof refractor;

const EXT_LANG: Record<string, string> = {
  js: "javascript", mjs: "javascript", cjs: "javascript", jsx: "javascript",
  ts: "typescript", tsx: "typescript", json: "json", md: "markdown", markdown: "markdown",
  css: "css", scss: "css", less: "css", html: "markup", xml: "markup", svg: "markup",
  sh: "bash", bash: "bash", zsh: "bash", py: "python", rs: "rust", go: "go", rb: "ruby",
  java: "java", c: "c", h: "c", cpp: "cpp", cc: "cpp", hpp: "cpp", cs: "csharp",
  yml: "yaml", yaml: "yaml", toml: "toml", sql: "sql", php: "php", dockerfile: "docker",
};

export function languageForPath(path: string): string | undefined {
  const name = path.split("/").pop()?.toLowerCase() ?? "";
  const ext = name.includes(".") ? name.split(".").pop()! : name;
  const lang = EXT_LANG[ext];
  return lang && refractor.registered(lang) ? lang : undefined;
}

/** Files with more changed lines than this skip word-level edit marking (perf). */
export const MARK_EDITS_MAX_CHANGES = 2000;

/**
 * Tokens for a file's hunks: syntax highlight (when the language is known)
 * plus word-level intra-line edit marks (when the diff isn't huge), or
 * undefined when neither applies.
 */
export function tokenizeFile(
  file: FileData,
  opts: { markEditsMaxChanges?: number } = {},
) {
  const language = languageForPath(fileDisplayPath(file));
  const { markEditsMaxChanges = MARK_EDITS_MAX_CHANGES } = opts;
  const { add, del } = countChanges(file);
  const wantEdits = add + del <= markEditsMaxChanges;
  if (!language && !wantEdits) return undefined;

  const base: TokenizeOptions = language
    ? { highlight: true, refractor: refractorCompat, language }
    : { highlight: false };
  if (wantEdits) {
    try {
      return tokenize(file.hunks, {
        ...base,
        enhancers: [markEdits(file.hunks, { type: "block" })],
      });
    } catch {
      // markEdits can throw on odd change blocks; retry highlight-only below.
    }
  }
  if (!language) return undefined;
  try {
    return tokenize(file.hunks, base);
  } catch {
    return undefined;
  }
}

export interface ChangeMeta {
  side: Side;
  line: number;
  hunk: string;
  /** The clicked line rendered as a diff line (e.g. "+    return …"). */
  lineText: string;
}

function diffLine(sign: string, content: string): string {
  return `${sign}${content}`;
}

export function fileDisplayPath(file: FileData): string {
  if (file.type === "delete") return file.oldPath;
  if (file.type === "rename") return file.newPath;
  return file.newPath || file.oldPath;
}

/**
 * Walk a file's hunks once and produce:
 *  - `metaByKey`: changeKey -> anchor (side/line) + containing hunk header,
 *     used when the user clicks a line to start a comment.
 *  - `keyByAnchor`: "SIDE:line" -> changeKey, used to place stored comments
 *     back onto their change.
 */
export function indexFile(file: FileData): {
  metaByKey: Map<string, ChangeMeta>;
  keyByAnchor: Map<string, string>;
} {
  const metaByKey = new Map<string, ChangeMeta>();
  const keyByAnchor = new Map<string, string>();

  for (const hunk of file.hunks) {
    for (const change of hunk.changes) {
      const key = getChangeKey(change);
      if (change.type === "normal") {
        metaByKey.set(key, {
          side: "RIGHT",
          line: change.newLineNumber,
          hunk: hunk.content,
          lineText: diffLine(" ", change.content),
        });
        keyByAnchor.set(`RIGHT:${change.newLineNumber}`, key);
        keyByAnchor.set(`LEFT:${change.oldLineNumber}`, key);
      } else if (change.type === "insert") {
        metaByKey.set(key, {
          side: "RIGHT",
          line: change.lineNumber,
          hunk: hunk.content,
          lineText: diffLine("+", change.content),
        });
        keyByAnchor.set(`RIGHT:${change.lineNumber}`, key);
      } else {
        metaByKey.set(key, {
          side: "LEFT",
          line: change.lineNumber,
          hunk: hunk.content,
          lineText: diffLine("-", change.content),
        });
        keyByAnchor.set(`LEFT:${change.lineNumber}`, key);
      }
    }
  }
  return { metaByKey, keyByAnchor };
}

export function changeKeyOf(change: ChangeData): string {
  return getChangeKey(change);
}

/**
 * Group `items` onto diff change keys by their `(side, line)` anchor, reusing
 * the same `"SIDE:line"` → changeKey map (`keyByAnchor`) that places local
 * comments. Items whose anchor isn't in the current diff (no side/line, or a
 * line not present — e.g. an outdated GitHub thread) fall into `orphans`.
 */
export function anchorByLine<T>(
  items: T[],
  sideLine: (t: T) => { side: string; line: number | null } | null,
  keyByAnchor: Map<string, string>,
): { byKey: Map<string, T[]>; orphans: T[] } {
  const byKey = new Map<string, T[]>();
  const orphans: T[] = [];
  for (const item of items) {
    const anchor = sideLine(item);
    const key =
      anchor && anchor.line != null
        ? keyByAnchor.get(`${anchor.side}:${anchor.line}`)
        : undefined;
    if (!key) {
      orphans.push(item);
      continue;
    }
    const arr = byKey.get(key) ?? [];
    arr.push(item);
    byKey.set(key, arr);
  }
  return { byKey, orphans };
}

const CONTEXT_LINES = 3;

/** The line number a change occupies on `side`, or null if it has no presence
 *  there (an insert has no LEFT line; a delete has no RIGHT line). */
function changeLineForSide(change: ChangeData, side: Side): number | null {
  if (change.type === "normal") {
    return side === "LEFT" ? change.oldLineNumber : change.newLineNumber;
  }
  if (change.type === "insert") return side === "RIGHT" ? change.lineNumber : null;
  return side === "LEFT" ? change.lineNumber : null; // delete
}

/** Render a parsed change back to its unified-diff line (" "/"+"/"-" prefix). */
function changeToDiffLine(change: ChangeData): string {
  const sign = change.type === "insert" ? "+" : change.type === "delete" ? "-" : " ";
  return `${sign}${change.content}`;
}

/**
 * A diff snippet for export: the hunk header plus the changes covering the
 * [lo, hi] selection on `side`, padded with up to `context` surrounding lines on
 * each end so a reader (or an AI) sees context, not just the commented line.
 * Falls back to the header alone if the selection isn't found in this hunk.
 */
export function hunkContextSnippet(
  hunk: HunkData,
  side: Side,
  lo: number,
  hi: number,
  context = CONTEXT_LINES,
): string {
  const { changes } = hunk;
  let first = -1;
  let last = -1;
  changes.forEach((c, i) => {
    const n = changeLineForSide(c, side);
    if (n != null && n >= lo && n <= hi) {
      if (first === -1) first = i;
      last = i;
    }
  });
  if (first === -1) return hunk.content;
  const start = Math.max(0, first - context);
  const end = Math.min(changes.length - 1, last + context);
  const lines = changes.slice(start, end + 1).map(changeToDiffLine);
  return [hunk.content, ...lines].join("\n");
}

/**
 * Build a synthetic single-file diff where the entire `source` is one hunk of
 * `normal` (unchanged) lines. Feeding this to `<Diff>`/`tokenizeFile`/`indexFile`
 * gives the full-file pane gutters, click-to-comment, widget injection and syntax
 * highlighting for free, while every line carries its absolute (head) number on
 * both sides.
 */
export function buildFullFileFile(path: string, source: string): FileData {
  // `git show`/the contents API append a trailing newline; drop the resulting
  // empty final element so we don't render a phantom blank last line.
  const lines = source.split("\n");
  if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();

  const changes: ChangeData[] = lines.map((content, i) => ({
    type: "normal",
    isNormal: true,
    content,
    oldLineNumber: i + 1,
    newLineNumber: i + 1,
  }));
  const hunk: HunkData = {
    content: `@@ -1,${lines.length} +1,${lines.length} @@`,
    oldStart: 1,
    newStart: 1,
    oldLines: lines.length,
    newLines: lines.length,
    changes,
  };
  return {
    type: "modify",
    oldPath: path,
    newPath: path,
    oldRevision: "",
    newRevision: "",
    oldEndingNewLine: true,
    newEndingNewLine: true,
    oldMode: "",
    newMode: "",
    hunks: [hunk],
  };
}

/** Head-side line numbers that are added or modified in `file`'s diff hunks. */
export function changedRightLines(file: FileData): Set<number> {
  const lines = new Set<number>();
  for (const hunk of file.hunks) {
    for (const change of hunk.changes) {
      if (change.type === "insert") lines.add(change.lineNumber);
    }
  }
  return lines;
}

/**
 * Number of real lines in a fetched source file. `git show`/the contents API
 * append a trailing newline, so split("\n") yields a final empty element that
 * must not count as a line (same convention as buildFullFileFile). "" → 0.
 */
export function sourceLineCount(source: string): number {
  if (source === "") return 0;
  const lines = source.split("\n");
  if (lines[lines.length - 1] === "") lines.pop();
  return lines.length;
}

/** Hidden old-side lines below the last hunk. 0 when the hunk reaches EOF; never negative. */
export function trailingGap(lastHunk: HunkData, oldLineCount: number): number {
  return Math.max(0, oldLineCount - (lastHunk.oldStart + lastHunk.oldLines) + 1);
}

/**
 * [start, end) old-side range revealing the n hidden lines directly ABOVE the
 * first hunk (the bottom of the hidden block, expanding upward). n = Infinity
 * reveals the whole leading block; oldStart === 1 yields the empty range [1, 1).
 */
export function leadingExpandRange(firstHunk: HunkData, n: number): [number, number] {
  return [Math.max(1, firstHunk.oldStart - n), firstHunk.oldStart];
}

/**
 * [start, end) old-side range revealing the n hidden lines directly BELOW the
 * last hunk (expanding downward). `end` is exclusive, so clamping to
 * `oldLineCount + 1` reaches EOF; a gap of 0 yields an empty range.
 */
export function trailingExpandRange(
  lastHunk: HunkData,
  oldLineCount: number,
  n: number,
): [number, number] {
  const start = lastHunk.oldStart + lastHunk.oldLines;
  return [start, Math.min(start + n, oldLineCount + 1)];
}

export function countChanges(file: FileData): { add: number; del: number } {
  let add = 0;
  let del = 0;
  for (const hunk of file.hunks) {
    for (const change of hunk.changes) {
      if (change.type === "insert") add++;
      else if (change.type === "delete") del++;
    }
  }
  return { add, del };
}
