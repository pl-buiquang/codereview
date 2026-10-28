import {
  getChangeKey,
  parseDiff,
  markEdits,
  tokenize,
  type ChangeData,
  type FileData,
  type HunkData,
  type TokenizeOptions,
} from "react-diff-view";
import { highlightToHast, isLanguageReady, isSupportedLanguage } from "./highlight";
import type { Side } from "./types";

// react-diff-view only calls `refractor.highlight(text, language)` and expects hast children;
// highlight.ts provides that shape from Shiki.
const highlighter = { highlight: highlightToHast } as never;

const EXT_LANG: Record<string, string> = {
  js: "javascript", mjs: "javascript", cjs: "javascript", jsx: "jsx",
  ts: "typescript", mts: "typescript", cts: "typescript", tsx: "tsx",
  json: "json", jsonc: "jsonc", md: "markdown", markdown: "markdown",
  css: "css", scss: "scss", less: "less", html: "html", htm: "html", xml: "xml", svg: "xml",
  csproj: "xml", vue: "vue", svelte: "svelte", erb: "erb",
  sh: "bash", bash: "bash", zsh: "bash", py: "python", rs: "rust", go: "go",
  rb: "ruby", rake: "ruby", gemspec: "ruby", ru: "ruby",
  java: "java", kt: "kotlin", kts: "kotlin", scala: "scala", swift: "swift", dart: "dart",
  c: "c", h: "c", cpp: "cpp", cc: "cpp", hpp: "cpp", cs: "csharp",
  yml: "yaml", yaml: "yaml", toml: "toml", ini: "ini", sql: "sql", php: "php", lua: "lua",
  graphql: "graphql", gql: "graphql", proto: "proto", diff: "diff", patch: "diff",
  ex: "elixir", exs: "elixir", tf: "hcl", hcl: "hcl", mk: "makefile",
};

/** Extension-less files (or ones whose name is the language). */
const NAME_LANG: Record<string, string> = {
  dockerfile: "docker", makefile: "makefile", gemfile: "ruby", rakefile: "ruby",
  guardfile: "ruby", podfile: "ruby", brewfile: "ruby", vagrantfile: "ruby",
};

export function languageForPath(path: string): string | undefined {
  const name = path.split("/").pop()?.toLowerCase() ?? "";
  const ext = name.includes(".") ? name.split(".").pop()! : name;
  const lang = NAME_LANG[name] ?? EXT_LANG[ext];
  return lang && isSupportedLanguage(lang) ? lang : undefined;
}

const FENCE_ALIAS: Record<string, string> = {
  shell: "bash", console: "bash", shellsession: "bash", sh: "bash",
  dockerfile: "docker", yml: "yaml", rb: "ruby", py: "python", ts: "typescript", js: "javascript",
};

/** Shiki language for a Markdown fence info string (`sh`, `ruby`, `ts`, …), if supported. */
export function languageForFence(info: string): string | undefined {
  const name = info.trim().toLowerCase();
  if (!name) return undefined;
  const lang = FENCE_ALIAS[name] ?? EXT_LANG[name] ?? name;
  return isSupportedLanguage(lang) ? lang : undefined;
}

/** Files with more changed lines than this skip word-level edit marking (perf). */
export const MARK_EDITS_MAX_CHANGES = 2000;

/**
 * Returns a copy of `hunks` where any change block with more than one deleted
 * line OR more than one inserted line has those changes neutralised to "normal"
 * type, preventing markEdits from pairing them. Single-line del+ins pairs are
 * left untouched so they still get intra-line character highlighting.
 */
function singleLinePairsHunks(hunks: readonly HunkData[]): HunkData[] {
  return hunks.map(hunk => {
    const changes: ChangeData[] = [];
    let i = 0;
    while (i < hunk.changes.length) {
      let j = i;
      while (j < hunk.changes.length && hunk.changes[j].type === "delete") j++;
      let k = j;
      while (k < hunk.changes.length && hunk.changes[k].type === "insert") k++;
      if (j > i || k > j) {
        const isMultiLine = j - i > 1 || k - j > 1;
        for (let x = i; x < k; x++) {
          changes.push(
            isMultiLine
              ? ({ ...hunk.changes[x], type: "normal" } as ChangeData)
              : hunk.changes[x],
          );
        }
        i = k;
      } else {
        changes.push(hunk.changes[i++]);
      }
    }
    return { ...hunk, changes };
  });
}

/**
 * Tokens for a file's hunks: syntax highlight (when the language is known)
 * plus word-level intra-line edit marks (when the diff isn't huge), or
 * undefined when neither applies.
 */
export function tokenizeFile(
  file: FileData,
  opts: { markEditsMaxChanges?: number } = {},
) {
  const lang = languageForPath(fileDisplayPath(file));
  // Grammars load lazily (useSyntaxReady re-runs callers once ready): plain until then.
  const language = lang && isLanguageReady(lang) ? lang : undefined;
  const { markEditsMaxChanges = MARK_EDITS_MAX_CHANGES } = opts;
  const { add, del } = countChanges(file);
  const wantEdits = add + del <= markEditsMaxChanges;
  if (!language && !wantEdits) return undefined;

  const base: TokenizeOptions = language
    ? { highlight: true, refractor: highlighter, language }
    : { highlight: false };
  if (wantEdits) {
    try {
      return tokenize(file.hunks, {
        ...base,
        enhancers: [markEdits(singleLinePairsHunks(file.hunks), { type: "block" })],
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
 * The current (RIGHT/head-side) text of lines [lo, hi], read from the rendered
 * hunks. Returns null unless EVERY line in the range is present (normal lines
 * use newLineNumber, inserts use lineNumber; deletes have no RIGHT presence).
 */
export function rightLinesText(
  hunks: HunkData[],
  lo: number,
  hi: number,
): string[] | null {
  const byLine = new Map<number, string>();
  for (const hunk of hunks) {
    for (const c of hunk.changes) {
      if (c.type === "normal") byLine.set(c.newLineNumber, c.content);
      else if (c.type === "insert") byLine.set(c.lineNumber, c.content);
    }
  }
  const out: string[] = [];
  for (let n = lo; n <= hi; n++) {
    const text = byLine.get(n);
    if (text === undefined) return null;
    out.push(text);
  }
  return out;
}

/** A ```suggestion fence around `lines`, lengthening the fence past any
 *  backtick run inside the content (min 3, GitHub-compatible). */
export function suggestionFence(lines: string[]): string {
  const longestRun = Math.max(
    0,
    ...lines.flatMap((l) => [...l.matchAll(/`+/g)].map((m) => m[0].length)),
  );
  const fence = "`".repeat(Math.max(3, longestRun + 1));
  return `${fence}suggestion\n${lines.join("\n")}\n${fence}`;
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

export function isLargeDiff(file: FileData, maxLines: number): boolean {
  const { add, del } = countChanges(file);
  return add + del > maxLines;
}

/** A one-hunk `FileData` from a GitHub `diffHunk` (the hunk header plus the
 *  lines up to the commented one), trimmed to its last `maxLines` changes. */
export function hunkSnippetFile(path: string, diffHunk: string, maxLines = 8): FileData | null {
  const text = `diff --git a/${path} b/${path}\n--- a/${path}\n+++ b/${path}\n${diffHunk}\n`;
  try {
    const file = parseDiff(text)[0];
    const hunk = file?.hunks[0];
    if (!hunk) return null;
    const changes = hunk.changes.slice(-maxLines);
    if (changes.length === 0) return null;
    return { ...file, hunks: [{ ...hunk, changes }] };
  } catch {
    return null;
  }
}
