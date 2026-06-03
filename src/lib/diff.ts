import { getChangeKey, tokenize, type ChangeData, type FileData } from "react-diff-view";
import { refractor } from "refractor";
import type { Side } from "./types";

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

/** Syntax-highlight tokens for a file's hunks, or undefined if unsupported. */
export function tokenizeFile(file: FileData) {
  const language = languageForPath(fileDisplayPath(file));
  if (!language) return undefined;
  try {
    return tokenize(file.hunks, { highlight: true, refractor, language });
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
