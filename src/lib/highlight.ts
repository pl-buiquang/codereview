import { useEffect, useSyncExternalStore } from "react";
import { createHighlighterCore, type HighlighterCore, type LanguageInput } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";
import githubLight from "shiki/themes/github-light-default.mjs";

/**
 * Syntax highlighting for diffs: Shiki (TextMate grammars) driven by GitHub's own scope→colour
 * table, with each of GitHub's colours collapsed onto one of our theme roles. Output is class-only
 * (`token tok-<role>`, coloured by the `--tok-*` vars), so every direction and the custom theme
 * keep working. Grammars load lazily; until one is ready, callers render plain text.
 */

const THEME = "github-light-default";

/** GitHub light's foreground colours → the theme role they stand for. Anything else is plain text. */
const ROLE_BY_COLOR: Record<string, string> = {
  "#6e7781": "comment",
  "#cf222e": "keyword",
  "#0550ae": "literal",
  "#953800": "variable",
  "#8250df": "function",
  "#116329": "literal", // tags, JSON keys: no role of their own, same as before
  "#0a3069": "string",
};

/** Grammar loaders by Shiki language id; each is its own chunk. */
const LANGS: Record<string, () => Promise<{ default: LanguageInput }>> = {
  bash: () => import("shiki/langs/bash.mjs"),
  c: () => import("shiki/langs/c.mjs"),
  cpp: () => import("shiki/langs/cpp.mjs"),
  csharp: () => import("shiki/langs/csharp.mjs"),
  css: () => import("shiki/langs/css.mjs"),
  dart: () => import("shiki/langs/dart.mjs"),
  diff: () => import("shiki/langs/diff.mjs"),
  docker: () => import("shiki/langs/docker.mjs"),
  elixir: () => import("shiki/langs/elixir.mjs"),
  erb: () => import("shiki/langs/erb.mjs"),
  go: () => import("shiki/langs/go.mjs"),
  graphql: () => import("shiki/langs/graphql.mjs"),
  hcl: () => import("shiki/langs/hcl.mjs"),
  html: () => import("shiki/langs/html.mjs"),
  ini: () => import("shiki/langs/ini.mjs"),
  java: () => import("shiki/langs/java.mjs"),
  javascript: () => import("shiki/langs/javascript.mjs"),
  json: () => import("shiki/langs/json.mjs"),
  jsonc: () => import("shiki/langs/jsonc.mjs"),
  jsx: () => import("shiki/langs/jsx.mjs"),
  kotlin: () => import("shiki/langs/kotlin.mjs"),
  less: () => import("shiki/langs/less.mjs"),
  lua: () => import("shiki/langs/lua.mjs"),
  makefile: () => import("shiki/langs/makefile.mjs"),
  markdown: () => import("shiki/langs/markdown.mjs"),
  php: () => import("shiki/langs/php.mjs"),
  proto: () => import("shiki/langs/proto.mjs"),
  python: () => import("shiki/langs/python.mjs"),
  ruby: () => import("shiki/langs/ruby.mjs"),
  rust: () => import("shiki/langs/rust.mjs"),
  scala: () => import("shiki/langs/scala.mjs"),
  scss: () => import("shiki/langs/scss.mjs"),
  sql: () => import("shiki/langs/sql.mjs"),
  svelte: () => import("shiki/langs/svelte.mjs"),
  swift: () => import("shiki/langs/swift.mjs"),
  toml: () => import("shiki/langs/toml.mjs"),
  tsx: () => import("shiki/langs/tsx.mjs"),
  typescript: () => import("shiki/langs/typescript.mjs"),
  vue: () => import("shiki/langs/vue.mjs"),
  xml: () => import("shiki/langs/xml.mjs"),
  yaml: () => import("shiki/langs/yaml.mjs"),
};

export function isSupportedLanguage(lang: string): boolean {
  return lang in LANGS;
}

let corePromise: Promise<HighlighterCore> | null = null;
let core: HighlighterCore | null = null;
const loaded = new Set<string>();
const pending = new Map<string, Promise<void>>();
const listeners = new Set<() => void>();

function getCore(): Promise<HighlighterCore> {
  corePromise ??= createHighlighterCore({
    themes: [githubLight],
    langs: [],
    engine: createJavaScriptRegexEngine(),
  }).then((c) => (core = c));
  return corePromise;
}

/** Loads a grammar (and the engine on first use). Resolves once `isLanguageReady(lang)`; never rejects. */
export function ensureLanguage(lang: string): Promise<void> {
  if (loaded.has(lang) || !isSupportedLanguage(lang)) return Promise.resolve();
  let p = pending.get(lang);
  if (!p) {
    p = getCore()
      .then(async (c) => {
        await c.loadLanguage((await LANGS[lang]()).default);
        loaded.add(lang);
        listeners.forEach((l) => l());
      })
      .catch(() => {})
      .finally(() => pending.delete(lang));
    pending.set(lang, p);
  }
  return p;
}

export function isLanguageReady(lang: string): boolean {
  return loaded.has(lang);
}

type HastText = { type: "text"; value: string };
type HastSpan = {
  type: "element";
  tagName: "span";
  properties: { className: string[] };
  children: HastText[];
};

/**
 * `refractor.highlight`-shaped result (hast children) for `text`, which react-diff-view turns into
 * per-line token trees. The grammar must be ready; throws otherwise.
 */
export function highlightToHast(text: string, lang: string): (HastText | HastSpan)[] {
  if (!core || !loaded.has(lang)) throw new Error(`grammar "${lang}" is not loaded`);
  const lines = core.codeToTokensBase(text, { lang, theme: THEME });
  const out: (HastText | HastSpan)[] = [];
  const pushText = (value: string) => {
    const last = out[out.length - 1];
    if (last?.type === "text") last.value += value;
    else out.push({ type: "text", value });
  };
  lines.forEach((line, i) => {
    if (i > 0) pushText("\n");
    for (const tok of line) {
      const role = tok.color ? ROLE_BY_COLOR[tok.color.toLowerCase()] : undefined;
      if (!role) {
        pushText(tok.content);
        continue;
      }
      out.push({
        type: "element",
        tagName: "span",
        properties: { className: ["token", `tok-${role}`] },
        children: [{ type: "text", value: tok.content }],
      });
    }
  });
  return out;
}

const subscribe = (cb: () => void) => {
  listeners.add(cb);
  return () => void listeners.delete(cb);
};

/**
 * True once `lang`'s grammar is loaded (kicking off the load on first use). Put the result in the
 * deps of whatever memoizes `tokenizeFile`, so highlighting appears when the grammar arrives.
 */
export function useSyntaxReady(lang: string | undefined): boolean {
  useEffect(() => {
    if (lang) void ensureLanguage(lang);
  }, [lang]);
  return useSyncExternalStore(subscribe, () => (lang ? loaded.has(lang) : false));
}
