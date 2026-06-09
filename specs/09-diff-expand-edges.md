# Spec 09 — Expand diff context to file top/bottom

Implements ROADMAP §1 "Expand diff context to file top/bottom". Frontend-only; no Rust changes.

## Problem

Only the gaps *between* hunks can be expanded today. The render prop in `FileBody`
(`src/components/ReviewView.tsx:915-937`) inserts an `ExpandControl` decoration only when
`prev != null && collapsed > 0` (line 922), and the comment at lines 917-918 explicitly defers the
rest:

```
// v1: only the gaps BETWEEN hunks get an expander; leading and
// trailing collapsed blocks (expand-to-top/bottom) are v1.1.
```

So a file whose first hunk starts at old line 200 hides lines 1-199 with no way to reveal them, and
the lines after the last hunk are likewise unreachable. All the machinery already exists:

- `ensureSource` (`ReviewView.tsx:524-537`) lazily fetches the LEFT/base file via
  `api.fileSource(reviewId, file.oldPath, "LEFT")` (`src/lib/api.ts:47-48`) and caches it in
  `rawSource` state.
- `expandBetween` (`ReviewView.tsx:539-549`) calls react-diff-view's
  `expandFromRawCode(hunks, src, start, end)` over **old-side** line numbers (`end` exclusive —
  verified in `node_modules/react-diff-view/src/utils/diff/expandCollapsedBlockBy.ts:120-131`).
- `getCollapsedLinesCountBetween(null, hunk)` already returns the leading gap size
  (`hunk.oldStart - 1`; `expandCollapsedBlockBy.ts:152-155`) and is already called with
  `prev = null` at `ReviewView.tsx:920`.
- Expanded hunks feed both rendering and anchoring (`indexFile` over `{ ...file, hunks }` at
  `ReviewView.tsx:551-555`), so revealed lines are automatically clickable/commentable.

The only genuinely missing input is the **trailing** gap size, which requires the base file's total
line count — known only once `rawSource` is fetched.

## Decisions (locked)

- **Frontend-only**, all changes in `src/components/ReviewView.tsx` plus small pure helpers (+ tests)
  in `src/lib/diff.ts`. No backend, no new dependencies.
- **Same gating as today**: edge expanders appear only where `canExpand` is true
  (`ReviewView.tsx:518-522` — local targets, not add/delete/binary). Do **not** widen this to
  GitHub-PR targets; that is Spec 10's job and this spec must not depend on it. When Spec 10 flips
  `canExpand` for PR targets, the edge expanders light up for free.
- **Trailing control renders optimistically.** Before `rawSource` is fetched the trailing gap size
  is unknown: show the control with no count (`count: null`). First click fetches the source
  (existing `ensureSource` pattern), the expansion recomputes against the real line count, and a
  zero-size gap no-ops — after which the control disappears because the count is now known to be 0.
  Rationale: no extra fetch on mount (keeps the current lazy-fetch behaviour), worst case is one
  dead click on files that already reach EOF.
- **Expand ranges are computed inside the `setHunks` updater** from the updater's own `h` argument
  (first/last hunk), never from captured state. `ensureSource` is `await`ed first, so hunks may have
  changed (another expander clicked) by the time the update runs; the existing `expandBetween`
  doesn't have this problem because its `prev`/`next` hunks come from the render prop, but edge
  ranges depend on the *current* first/last hunk.
- **"All" is expressed as `n = Number.POSITIVE_INFINITY`** and the range helpers clamp
  (top clamps `start` to 1, bottom clamps `end` to `lineCount + 1`). One code path for chunk and
  all; no separate "expand everything" branch.
- **Chunk semantics mirror GitHub**: the leading chunk reveals the `n` lines *adjacent to* the first
  hunk (the bottom of the hidden block, expanding upward); the trailing chunk reveals the `n` lines
  just below the last hunk (expanding downward). Chunk size stays `EXPAND_CHUNK = 20`
  (`ReviewView.tsx:41`).
- **Reuse `ExpandControl`** (`ReviewView.tsx:944-974`) with two additive props (`count: number | null`,
  `direction?: "up" | "down" | "between"`) rather than a new component. Existing CSS classes
  (`.diff-expander`, `.expand-control`, `.expand-btn`) are reused; no stylesheet changes required.

## Design

### 1. Pure helpers — `src/lib/diff.ts` (NEW exports)

All math lives here so vitest covers the off-by-ones without rendering. Old-side line numbers
throughout; ranges are `[start, end)` exactly as `expandFromRawCode` expects.

```ts
/**
 * Number of real lines in a fetched source file. `git show`/the contents API
 * append a trailing newline, so split("\n") yields a final empty element that
 * must not count as a line (same convention as buildFullFileFile, diff.ts:204-208).
 * "" → 0.
 */
export function sourceLineCount(source: string): number;

/** Hidden old-side lines below the last hunk. 0 when the hunk reaches EOF; never negative. */
export function trailingGap(lastHunk: HunkData, oldLineCount: number): number;
// = Math.max(0, oldLineCount - (lastHunk.oldStart + lastHunk.oldLines) + 1)

/** [start, end) revealing the n hidden lines directly ABOVE the first hunk. */
export function leadingExpandRange(firstHunk: HunkData, n: number): [number, number];
// = [Math.max(1, firstHunk.oldStart - n), firstHunk.oldStart]
// n = Infinity → [1, oldStart] (whole leading block). oldStart === 1 → empty range [1, 1).

/** [start, end) revealing the n hidden lines directly BELOW the last hunk. */
export function trailingExpandRange(
  lastHunk: HunkData,
  oldLineCount: number,
  n: number,
): [number, number];
// start = lastHunk.oldStart + lastHunk.oldLines
// end   = Math.min(start + n, oldLineCount + 1)   // Infinity-safe; end exclusive ⇒ +1 reaches EOF
// gap of 0 ⇒ start === end (empty range).
```

The leading gap *count* needs no new helper: keep using
`getCollapsedLinesCountBetween(null, firstHunk)` (already imported and already called with `null`).

### 2. `FileReview` — new callbacks + derived line count (`ReviewView.tsx`, near `expandBetween` at 539-549)

```ts
const oldLineCount = useMemo(
  () => (rawSource == null ? null : sourceLineCount(rawSource)),
  [rawSource],
);

const expandLeading = useCallback(
  async (n: number) => {
    const src = await ensureSource();
    if (src == null) return;
    setHunks((h) => {
      if (h.length === 0) return h;
      const [start, end] = leadingExpandRange(h[0], n);
      return start < end ? expandFromRawCode(h, src, start, end) : h;
    });
  },
  [ensureSource],
);

const expandTrailing = useCallback(
  async (n: number) => {
    const src = await ensureSource();
    if (src == null) return;
    setHunks((h) => {
      if (h.length === 0) return h;
      const [start, end] = trailingExpandRange(h[h.length - 1], sourceLineCount(src), n);
      return start < end ? expandFromRawCode(h, src, start, end) : h;
    });
  },
  [ensureSource],
);
```

Import `sourceLineCount`, `trailingGap`, `leadingExpandRange`, `trailingExpandRange` from
`../lib/diff` (extend the existing import block at `ReviewView.tsx:18-26`).

### 3. `FileBody` — render the edge expanders (`ReviewView.tsx:830-942`)

New props threaded from `FileReview` (add to both the JSX at ~805-825 and the prop types at
~849-868):

```ts
oldLineCount: number | null;                  // null until rawSource fetched
onExpandLeading: (n: number) => void;
onExpandTrailing: (n: number) => void;
```

Inside the `<Diff>` render prop, replace the v1/v1.1 comment (lines 917-918) and extend the
`flatMap`:

- **Leading** — at `i === 0`, before the hunk row, when
  `canExpand && getCollapsedLinesCountBetween(null, hunk) > 0`:

  ```tsx
  <Decoration key="exp-top" className="diff-expander">
    <ExpandControl
      count={getCollapsedLinesCountBetween(null, hunk)}
      direction="up"
      busy={expanding}
      onExpandChunk={() => onExpandLeading(EXPAND_CHUNK)}
      onExpandAll={() => onExpandLeading(Number.POSITIVE_INFINITY)}
    />
  </Decoration>
  ```

- **Between** — unchanged (existing block at 922-933; pass `direction="between"` implicitly via the
  prop default).

- **Trailing** — at `i === renderedHunks.length - 1`, after the hunk row. Gate:
  `canExpand && (oldLineCount == null || trailingGap(hunk, oldLineCount) > 0)`:

  ```tsx
  <Decoration key="exp-bottom" className="diff-expander">
    <ExpandControl
      count={oldLineCount == null ? null : trailingGap(hunk, oldLineCount)}
      direction="down"
      busy={expanding}
      onExpandChunk={() => onExpandTrailing(EXPAND_CHUNK)}
      onExpandAll={() => onExpandTrailing(Number.POSITIVE_INFINITY)}
    />
  </Decoration>
  ```

Widget placement (unified view; same row order in split view):

```
┌──────────────────────────────────────────────────────┐
│ src/foo.ts                          +12 −3  [Viewed]  │  ← diff-file-header (unchanged)
├──────────────────────────────────────────────────────┤
│ ⋯ 199 hidden lines        [↑ 20 lines] [all]          │  ← NEW leading expander
│ @@ -200,7 +200,8 @@                                   │
│   …hunk 1…                                            │
│ ⋯ 41 hidden lines         [20 lines] [all]            │  ← existing between expander
│ @@ -248,5 +249,5 @@                                   │
│   …hunk 2 (last)…                                     │
│ ⋯ hidden lines            [↓ 20 lines] [all]          │  ← NEW trailing expander
│   (count appears once the base source is fetched)     │
└──────────────────────────────────────────────────────┘
```

### 4. `ExpandControl` — additive props (`ReviewView.tsx:944-974`)

```ts
function ExpandControl({
  count,            // number | null — null: size unknown (trailing, source not yet fetched)
  busy,
  direction = "between", // "up" | "down" | "between" — arrow on the chunk button
  onExpandChunk,
  onExpandAll,
}: { count: number | null; busy: boolean; direction?: "up" | "down" | "between";
     onExpandChunk: () => void; onExpandAll: () => void })
```

Rendering rules (keep the existing structure/classes):

- Label: `⋯ {count} hidden lines` when `count != null`, else `⋯ hidden lines`.
- Chunk button: shown when `count == null || count > EXPAND_CHUNK` (today's `count > EXPAND_CHUNK`
  rule at line 962, plus the unknown case). Text: `↑ 20 lines` / `↓ 20 lines` / `20 lines` per
  `direction`.
- "all" button: unchanged, always shown.
- `busy` ("Expanding…") branch: unchanged. All three controls share the one `expanding` flag — same
  as today.

Existing between-gap call sites (lines 925-930) compile unchanged (`direction` defaults to
`"between"`; `count` widens to `number | null`).

### Data flow recap

```
click edge ExpandControl
  → onExpandLeading/Trailing(n)               (FileBody → FileReview callback)
  → ensureSource()                            (cached after first call; api.fileSource LEFT)
  → setHunks(h => expandFromRawCode(h, src, …range from h's first/last hunk…))
  → hunks state change re-derives metaByKey/keyByAnchor/tokens (551-555)
  → revealed lines render, are clickable, and anchor comments — identical to between-gap behaviour
```

Ephemeral by design, exactly like between-gap expansion: a refetched diff resets `hunks` via the
`useEffect` at `ReviewView.tsx:510`.

## Tasks

1. **`src/lib/diff.ts`**: add `sourceLineCount`, `trailingGap`, `leadingExpandRange`,
   `trailingExpandRange` with doc comments. (Buildable alone.)
2. **`src/lib/diff.test.ts`**: unit tests for the four helpers + the two
   `expandFromRawCode`-integration cases (see matrix). Extend the existing
   `"context expansion anchoring"` describe block (line 243) and reuse its `BASE_SOURCE`/`parseDiff`
   fixtures.
3. **`ReviewView.tsx` — `ExpandControl`**: widen `count` to `number | null`, add `direction` prop
   and the arrow/label/chunk-visibility rules. (Buildable alone; existing call sites unaffected.)
4. **`ReviewView.tsx` — `FileReview` + `FileBody`**: add `expandLeading`/`expandTrailing`/
   `oldLineCount`, thread the three new props, render the leading/trailing `Decoration`s in the
   render prop, delete the v1.1 comment (lines 917-918) and replace it with a short note that all
   three gap kinds now expand.
5. Run gates + manual verify.

## Test matrix (vitest, `src/lib/diff.test.ts`)

| Test | Asserts |
|---|---|
| `sourceLineCount` trailing newline | `"a\nb\n"` → 2; `"a\nb"` → 2; `""` → 0; `"\n"` → 1 |
| `trailingGap` basic | last hunk `oldStart=10, oldLines=5`, count 30 → 16 (`30 - 14`); count 14 → 0 |
| `trailingGap` clamps | count smaller than hunk end → 0, never negative |
| `leadingExpandRange` chunk | `oldStart=100, n=20` → `[80, 100]` |
| `leadingExpandRange` clamps to top | `oldStart=10, n=20` → `[1, 10]`; `Infinity` → `[1, 10]` |
| `leadingExpandRange` no gap | `oldStart=1` → `[1, 1]` (empty range — caller no-ops) |
| `trailingExpandRange` chunk | `oldStart=10, oldLines=5`, count 100, n=20 → `[15, 35]` |
| `trailingExpandRange` all / clamp to EOF | same hunk, `n=Infinity` → `[15, 101]` (end exclusive reaches line 100) |
| `trailingExpandRange` no gap | count 14 → `[15, 15]` (empty) |
| leading expansion integrates | `parseDiff` a fixture whose first hunk starts past line 1; `expandFromRawCode(hunks, BASE_SOURCE, ...leadingExpandRange(first, Infinity))`; assert old line 1 is now a `normal` change with the correct `oldLineNumber`/`newLineNumber` pair, and `indexFile` yields a `RIGHT:<newLine>` anchor key for a revealed line (revealed lines must stay commentable) |
| trailing expansion integrates | expand `trailingExpandRange(last, sourceLineCount(BASE_SOURCE), Infinity)`; assert the final source line appears as a `normal` change and no phantom empty line is appended past EOF |

No Rust tests: zero backend changes.

## Gates

1. `pnpm exec tsc --noEmit`
2. `pnpm build`
3. `pnpm test` (vitest run)
4. `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`
5. `cargo test --manifest-path src-tauri/Cargo.toml`

(4 and 5 must still pass untouched — they prove no accidental backend drift.)

## Manual verify (`pnpm tauri dev`)

Use a local repo with a file of 100+ lines; create a branch that edits a line around the middle
**and** a line near (but not at) the end, so the diff has a deep leading block and a small trailing
one.

1. Open a virtual-PR review (base = main, head = the branch) on that repo.
2. The file shows `⋯ N hidden lines [↑ 20 lines] [all]` above the first hunk and
   `⋯ hidden lines [↓ 20 lines] [all]` (no count yet) below the last.
3. Click `↑ 20 lines` repeatedly: 20 lines above the hunk reveal each time, the count decreases,
   and the control disappears once line 1 is visible. Line numbers in both gutters are continuous.
4. Click the trailing `all`: the file reveals to its true last line (compare with "View file"), no
   trailing blank phantom line, and the control disappears. After this first fetch the leading
   control (if re-collapsed via a diff refetch) and trailing control show exact counts.
5. Click a newly revealed top line and add a comment; confirm it lands on that line, persists after
   closing/reopening the review, and shows in the export.
6. Edit a file so its last hunk already touches EOF: the trailing control shows without a count,
   one click fetches the source, nothing expands, and the control disappears.
7. Open a **GitHub PR** review: no leading/trailing expanders appear anywhere (`canExpand` false),
   matching today's between-gap behaviour.

## Out of scope

- **GitHub-PR targets** — `canExpand` stays `kind === "local"`; enabling PR targets (populating
  `base_sha`, backfill) is Spec 10.
- Added/deleted/binary files (no base side; already excluded by `canExpand`).
- Files whose diff parses to zero hunks — no expander is rendered (the render prop never runs);
  don't add a special case.
- Persisting expansion state across diff refetches/app restarts (expansion is ephemeral by design,
  `ReviewView.tsx:506-510`).
- Eagerly fetching the base source on mount just to show the trailing count, hover prefetch, or any
  fetch-coalescing beyond the existing `ensureSource` cache.
- Right/head-side source expansion, `markEdits` word highlighting, keyboard shortcuts, CSS redesign
  of the expander row.
