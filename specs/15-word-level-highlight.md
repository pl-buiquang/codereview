# Spec 15 — Word-level intra-line highlighting (`markEdits`)

Implements ROADMAP §1 — "**Word-level intra-line highlighting** — `markEdits` from
`react-diff-view`" (`ROADMAP.md:31`).

## Problem

When a line is modified, the diff paints the whole `-` line red and the whole `+` line green;
nothing shows *which characters* changed. GitHub highlights the edited sub-ranges inside the
line pair, which is what `react-diff-view` ships as the `markEdits` tokenize enhancer — and we
already pay for the tokenize pass.

Evidence:

- `src/lib/diff.ts:39-47` — `tokenizeFile` calls `tokenize(file.hunks, { highlight: true,
  refractor: refractorCompat, language })` with **no `enhancers`**, so no edit marks are ever
  produced. It also returns `undefined` when the language is unknown, skipping tokenization
  entirely.
- `src/styles.css:574-579` — theme rules for `.diff-code-insert .diff-code-edit` /
  `.diff-code-delete .diff-code-edit` **already exist** (color-mix of `--success`/`--danger`)
  but are dead CSS today because no `edit` tokens are emitted.
- `src/lib/diff.ts:4` — only `tokenize` is imported from `react-diff-view`; `markEdits` is
  unused.

### Verified package API (react-diff-view 3.3.3, from node_modules — do not re-guess)

- `node_modules/react-diff-view/package.json` — version `3.3.3`; `diff-match-patch` is its own
  dependency (markEdits uses it internally with `diff_cleanupSemantic`), nothing to add to ours.
- `node_modules/react-diff-view/types/index.d.ts:5` — `markEdits` is exported from the package
  root: `export { markEdits, markWord, pickRanges, tokenize } from './tokenize';`
- `node_modules/react-diff-view/types/tokenize/markEdits.d.ts` —
  `markEdits(hunks: HunkData[], { type }?: MarkEditsOptions): TokenizeEnhancer` with
  `MarkEditsType = 'block' | 'line'`; default is `'block'`
  (`src/tokenize/markEdits.ts:170` in the package).
- `node_modules/react-diff-view/types/tokenize/index.d.ts:10-11` —
  `TokenizeOptions = ToTokenTreeOptions & { enhancers?: TokenizeEnhancer[] }`, so the call shape
  is `tokenize(hunks, { …, enhancers: [markEdits(hunks)] })`.
- `node_modules/react-diff-view/types/tokenize/toTokenTrees.d.ts` — `ToTokenTreeOptions` is a
  union: `{ highlight?: false }` (no language needed) **or**
  `{ highlight: true, refractor, language }`. So tokenizing *without* syntax highlight is legal —
  needed to mark edits in files whose language isn't registered.
- `node_modules/react-diff-view/src/Hunk/CodeCell.tsx:20` — an `edit` token renders as
  `<span class="diff-code-edit">…</span>`.
- `node_modules/react-diff-view/style/index.css` (imported in `src/main.tsx:5`, *before*
  `./styles.css` on line 6) — defines `.diff-code-edit` plus
  `.diff-code-insert .diff-code-edit` / `.diff-code-delete .diff-code-edit` backed by
  `--diff-code-insert-edit-background-color` (#c0dc91) / `--diff-code-delete-edit-background-color`
  (#f39ea2). Our `styles.css:574-579` rules have equal specificity and load later, so the dark
  theme wins. **No CSS changes are required.**
- `node_modules/react-diff-view/src/tokenize/markEdits.ts:136` — `markEdits` can `throw new
  Error('Could not find start line number for edit')` on pathological change blocks; the
  enhancer must be fallback-guarded, not trusted.

## Decisions (locked)

- **`type: 'block'` (the default).** Block mode diffs each whole insert/delete run as one text
  blob (GitHub-like, handles unbalanced/multi-line edits); `'line'` only pairs adjacent
  delete/insert lines. Pass `{ type: 'block' }` explicitly so the choice is visible.
- **Edit marks apply even when the language is unregistered.** `tokenizeFile` no longer bails to
  `undefined` for unknown extensions; it tokenizes with `highlight: false` + the enhancer.
  Rationale: word-level marks are language-agnostic and the union type explicitly supports it.
- **Perf guard: skip `markEdits` when `add + del > 2000`** (reuse `countChanges`,
  `src/lib/diff.ts:250-260`). Highlight-only tokenization is kept in that case. The threshold is
  an exported constant and a test-only override parameter — **no settings UI**.
- **Two-stage fallback.** If `tokenize` with the enhancer throws, retry without the enhancer
  (keep syntax highlight); only then fall back to `undefined`. Today's single `try/catch` would
  drop syntax highlighting whenever `markEdits` hiccups.
- **No caller changes.** All three call sites — `src/components/DiffViewer.tsx:39`,
  `src/components/ReviewView.tsx:555` (FileReview, expanded hunks included), and
  `src/components/FileViewPane.tsx:87` (synthetic all-normal full-file, where `markEdits` is a
  cheap no-op: `findChangeBlocks` finds no non-normal change) — get marks for free through
  `tokenizeFile`.
- **Frontend-only.** No Rust, no DB, no migration.

## Design

### Widget placement (rendering effect only — no new widget)

```
 12  12   const TIMEOUT = 1000;            ← context, unchanged
 13      - const limit = ▓getLimit(cfg)▓;  ← .diff-code-delete, edited span darker red
     13  + const limit = ▓cfg.limit ?? 50▓;← .diff-code-insert, edited span darker green
 14  14   return limit;
```

The `▓…▓` ranges are the `<span class="diff-code-edit">` children that `markEdits` injects into
the token stream; everything else renders exactly as today.

### `src/lib/diff.ts` — the only source file that changes

Imports (line 1-7): add `markEdits` and `type TokenizeOptions` to the existing `react-diff-view`
import.

Replace `tokenizeFile` (`src/lib/diff.ts:39-47`) with:

```ts
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
```

Notes for the implementer:

- `markEdits` must receive **the same hunks** passed to `tokenize`. Callers that expand context
  (`ReviewView.tsx:555` tokenizes `{ ...file, hunks }`) are unaffected because `tokenizeFile`
  reads `file.hunks` for both.
- Expansion only adds `normal` changes, so `countChanges` over expanded hunks equals the
  original diff's count — the guard is stable across expansion.
- `refractorCompat` (`src/lib/diff.ts:16-20`) stays exactly as is; the refractor-v5 unwrap is
  orthogonal to enhancers.
- Keep the function signature backward compatible: every existing caller passes only `file`.

### Files touched

| File | Change |
|---|---|
| `src/lib/diff.ts` | import `markEdits`/`TokenizeOptions`; new `MARK_EDITS_MAX_CHANGES` const; rewrite `tokenizeFile` as above |
| `src/lib/diff.test.ts` | new `tokenizeFile` describe block (see test matrix) |
| `src/styles.css` | **no change** — verify `:574-579` renders well in dev; only touch if visibly broken |

### Data flow

`review_diff` text → `parseDiff` → `FileData` → `tokenizeFile` (now: refractor tokens ⊕
`markEdits` ranges) → `tokens` prop on `<Diff>` → `CodeCell` renders `edit` token nodes as
`span.diff-code-edit` → themed by existing `styles.css:574-579`.

## Tasks

1. **Wire `markEdits` into `tokenizeFile`** (`src/lib/diff.ts`): imports, `MARK_EDITS_MAX_CHANGES`,
   threshold guard, no-language path, two-stage fallback. Buildable alone; all callers inherit it.
2. **Add vitest coverage** in `src/lib/diff.test.ts` (matrix below).
3. **Visual pass** in `pnpm tauri dev` (manual verify below); adjust `styles.css:574-579` only if
   the marks are illegible against the theme.

## Test matrix (vitest — `src/lib/diff.test.ts`; frontend-only spec, no Rust tests)

Add a tree-walk helper local to the test file (tokens are `HunkTokens { old, new:
TokenNode[][] }`; collect `node.type` recursively through `children`):

```ts
function tokenTypes(linesOfTrees: TokenNode[][]): Set<string>
```

Reuse the existing fixture style (`parseDiff` over inline diff strings, as at
`src/lib/diff.test.ts:59-67`). Use a `.ts` file path in fixtures so `languageForPath` resolves.

| Test | Assert |
|---|---|
| `marks intra-line edits on a modified line pair` | diff with `-const a = 1;` / `+const a = 2;` in `f.ts` → tokens defined; `tokenTypes(tokens.new)` and `tokenTypes(tokens.old)` both contain `"edit"` |
| `keeps syntax highlight tokens alongside edit marks` | same fixture → `tokenTypes(tokens.new)` contains a non-`text`, non-`edit` type (refractor node) |
| `skips markEdits above the changed-line threshold` | same fixture (add+del = 2), `tokenizeFile(file, { markEditsMaxChanges: 1 })` → tokens **defined** (highlight kept) but no `"edit"` in either side |
| `marks edits in files without a registered language` | modified line in `notes.unknownext` → tokens defined; contains `"edit"` |
| `returns undefined when neither language nor edits apply` | `notes.unknownext` with `{ markEditsMaxChanges: 0 }` → `undefined` |
| `adds no edit marks for pure insert/delete blocks` | diff that only inserts a line into `f.ts` → tokens defined, no `"edit"` (markEdits skips single-sided blocks: `diffText` returns empty for ≤1 diff) |
| existing suites | `languageForPath` / `indexFile` / `countChanges` / expansion tests unchanged and still green |

## Gates

- `pnpm exec tsc --noEmit`
- `pnpm build`
- `pnpm test` (vitest run)
- `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`
- `cargo test --manifest-path src-tauri/Cargo.toml`

No spec-specific extra gate (Rust untouched; clippy/cargo-test must simply stay green).

## Manual verify

1. `pnpm tauri dev`; open (or create) a **local** virtual-PR review whose diff modifies existing
   lines (a rename of one identifier on a long line is ideal).
2. In a modified `-`/`+` line pair: only the changed sub-range shows the darker red/green
   (inspect: it is a `span.diff-code-edit` inside `td.diff-code-delete` / `td.diff-code-insert`).
3. Confirm syntax highlighting still renders on the same lines (refractor colors inside and
   outside the mark).
4. Toggle split/unified view — marks render in both (`CodeCell` is shared).
5. Open a file with an unmapped extension (e.g. a `Makefile`) containing a modified line — edit
   marks appear even though there's no syntax color.
6. Expand a collapsed gap in a local review (gap expander) — marks on nearby changed lines
   survive re-tokenization.
7. Open the full-file pane (FileViewPane) — renders exactly as before (all-normal synthetic
   hunks produce no marks).
8. Open a GitHub PR review (`gh` authenticated) — marks appear there too (same `tokenizeFile`
   path).

## Out of scope

- `markWord` (whitespace/tab visualization enhancer) — separate concern.
- A settings toggle or per-file override for the threshold / mark type.
- `type: 'line'` mode, or any heuristic switching between block/line.
- Offloading tokenization to a web worker (revisit only if profiling shows jank).
- Any change to `DiffViewer.tssx`/`ReviewView.tsx`/`FileViewPane.tsx` call sites, CSS variables,
  or the vendor stylesheet.
- Backend/Rust changes of any kind.
