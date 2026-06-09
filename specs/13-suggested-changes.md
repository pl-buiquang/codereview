# Spec 13 — Suggested changes (```suggestion blocks)

Implements ROADMAP §1 — "**Suggested changes** — GitHub-style ```suggestion blocks that publish as
suggestions."

## Problem

There is no way to propose a concrete replacement for the commented line(s), GitHub's most-used
review affordance:

- The composer (`Composer`, `src/components/ReviewView.tsx:1134-1173`) is a bare textarea — nothing
  helps the user author the exact ```` ```suggestion ```` fence GitHub expects, and nothing
  prefills the *current* text of the selected lines (which the user must reproduce verbatim for a
  suggestion to make sense).
- `src/components/Markdown.tsx` renders every fenced code block through the default
  `react-markdown` `pre`/`code`, so a ```` ```suggestion ```` block previews as a plain code block
  labeled nothing — the user can't tell it will become a "Suggested change" on GitHub. The same
  applies to suggestion fences in *fetched* PR threads (`GithubThread` renders bodies through
  `Markdown` too).

The pipeline after composition already works and needs **zero backend changes**:

- Publish: `build_publish_payload` (`src-tauri/src/commands/review.rs:709-750`) sends `c.body`
  verbatim as the inline comment `body` (line 728); GitHub natively turns a suggestion fence on a
  RIGHT-side line/range comment into an applyable suggestion.
- Export: `render_markdown` (`src-tauri/src/export.rs:72`) and `render_json`
  (`src-tauri/src/export.rs:96`) emit `c.body` verbatim — the fence survives untouched (the body is
  never nested inside another fence; only `diff_hunk` is, at `export.rs:67-69`).

## Decisions (locked)

- **RIGHT-side line/range diff comments only.** The "Insert suggestion" button appears only when
  the anchor is `side === "RIGHT"`, `subject_type === "line"`, `origin === "diff"`. Suggestions on
  LEFT/deleted lines and file-level comments are meaningless to GitHub; file-view comments fold
  into the review body on publish where GitHub renders no suggestion UI.
- **Seed comes from the in-scope hunk data.** `FileReview` already holds the live `hunks` state
  (`ReviewView.tsx:509`, including user-expanded context lines), and `change.content` in
  react-diff-view hunks is the raw line text *without* the `+`/`-`/space prefix — read the selected
  RIGHT lines straight from there. No backend call.
- **If any selected line can't be resolved from the rendered hunks, no button.** Hidden, not
  disabled. Covers collapsed-context gaps and outdated comments (see next).
- **Outdated comments get no seed.** For existing comments, require
  `anchored_head_sha == null || anchored_head_sha === target.head_sha` (mirrors `is_anchored_to`,
  `review.rs:702-707`) — a stale anchor would seed the *wrong* current text.
- **Insertion appends at the end of the current text** (with a blank-line separator when non-empty).
  Cursor-position insertion needs ref plumbing through two components for marginal gain — v1.1 if
  ever.
- **Fence length adapts to content:** if any seeded line contains a backtick run ≥ 3, use a fence
  one backtick longer than the longest run (min 3) — same trick GitHub uses, so code-about-markdown
  is suggestible. `react-markdown` still yields `language-suggestion` for longer fences.
- **Render v1 = labeled "new side only" panel.** The Markdown renderer receives no hunk context
  (it renders bodies everywhere: previews, exports preview, GitHub threads), so v1 does **not**
  attempt an old-vs-new mini diff. A `Suggested change` header + green-tinted body. An *empty*
  suggestion body (GitHub semantics: delete the lines) renders a muted "(removes the selected
  lines)" placeholder instead of an empty box.
- **Publish/export pass-through, no backend change.** Lock with one Rust regression test that a
  suggestion fence in a comment body survives `render_markdown` verbatim.
- **No FileViewPane button.** Its composer creates `origin = 'file_view'` comments
  (`FileViewPane.tsx:143-157`) which never publish inline; the optional props are simply not passed
  there.
- **No new dependencies, no DB change** (reserved migrations 0007/0008 are untouched).

## Design

### 1. Seed helpers — `src/lib/diff.ts`

Pure, vitest-covered. Add below `hunkContextSnippet`:

```ts
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
```

### 2. Compose wiring — `src/components/ReviewView.tsx`

Data flow (all props optional so other `CommentItem`/`LineWidget` call sites — orphans, file
comments, `FileViewPane` — are untouched):

```
FileReview (owns hunks state + selection)
  ├─ composerSuggestionSeed: string | null        ── selection → rightLinesText → suggestionFence
  ├─ suggestionSeedFor(c: Comment): string | null ── comment anchor → same helpers
  ▼
LineWidget({ ..., composerSuggestionSeed?, suggestionSeedFor? })
  ├─ Composer({ ..., suggestionSeed?: string | null })       → button in composer-actions
  └─ CommentItem({ ..., suggestionSeed?: string | null })    → button in the Write tab
```

In `FileReview` (next to `selectedChanges`, ~`ReviewView.tsx:647`):

```ts
const composerSuggestionSeed = useMemo(() => {
  if (!selection || !range || selection.side !== "RIGHT") return null;
  const lines = rightLinesText(hunks, range.lo, range.hi);
  return lines ? suggestionFence(lines) : null;
}, [selection, range, hunks]);

const suggestionSeedFor = useCallback(
  (c: Comment): string | null => {
    if (c.side !== "RIGHT" || c.subject_type !== "line" || c.origin !== "diff") return null;
    if (c.anchored_head_sha && c.anchored_head_sha !== detail.target.head_sha) return null;
    const lines = rightLinesText(hunks, c.start_line ?? c.line, c.line);
    return lines ? suggestionFence(lines) : null;
  },
  [hunks, detail.target.head_sha],
);
```

Pass both into the `widgets[key]` `LineWidget` (~`ReviewView.tsx:707-720`). `LineWidget` forwards
`suggestionSeed={suggestionSeedFor?.(c) ?? null}` to each `CommentItem` and
`suggestionSeed={composerSuggestionSeed}` to `Composer`.

`Composer` (`ReviewView.tsx:1134`): new optional prop `suggestionSeed?: string | null`. When
non-null, render in `composer-actions` (left-aligned, before Cancel):

```tsx
<button
  className="suggest-btn"
  title="Insert a suggestion block prefilled with the current line(s)"
  onClick={() => setText((t) => (t.trim() === "" ? suggestionSeed : `${t}\n\n${suggestionSeed}`))}
>
  ± Insert suggestion
</button>
```

`CommentItem` (`ReviewView.tsx:1022`): same optional prop; render the same button inside the Write
tab (under the tab strip, only when `tab === "write"` and `!readOnly`), appending to `body` the
same way **and calling the debounced `save(next)`** so autosave fires.

Widget placement sketch:

```
│ 12 │ +  const x = compute(y);          ← selected RIGHT lines
│ 13 │ +  return x;
├────┴───────────────────────────────────────────────┤
│ Lines 12–14 (RIGHT)                                 │
│ ┌─────────────────────────────────────────────────┐ │
│ │ Leave a comment…                                │ │
│ └─────────────────────────────────────────────────┘ │
│ [± Insert suggestion]          [Cancel] [Add comment] │
└─────────────────────────────────────────────────────┘
```

### 3. Render — `src/components/Markdown.tsx`

Override `pre` (block fences arrive as `<pre><code class="language-suggestion">`; inline code never
hits `pre`):

```tsx
pre: ({ children, ...props }) => {
  const child = Array.isArray(children) ? children[0] : children;
  if (
    isValidElement(child) &&
    typeof (child.props as { className?: string }).className === "string" &&
    (child.props as { className: string }).className.includes("language-suggestion")
  ) {
    const code = (child.props as { children?: React.ReactNode }).children;
    const empty = typeof code === "string" && code.trim() === "";
    return (
      <div className="suggestion-block">
        <div className="suggestion-block-header">Suggested change</div>
        {empty ? (
          <p className="muted suggestion-block-empty">(removes the selected lines)</p>
        ) : (
          <pre className="suggestion-block-new">
            <code>{code}</code>
          </pre>
        )}
      </div>
    );
  }
  return <pre {...props}>{children}</pre>;
},
```

This upgrades every `Markdown` call site for free — comment previews, the review-body preview,
*and* fetched GitHub threads (`GithubThread.tsx`) whose authors used suggestions.

### 4. Styles — `src/styles.css`

Add near the `.composer-*` rules (~line 1266): `.suggestion-block` (bordered, rounded),
`.suggestion-block-header` (small-caps/muted strip), `.suggestion-block-new` (green addition tint,
e.g. `background: rgba(46, 160, 67, 0.15)`, monospace, no extra margin), `.suggestion-block-empty`,
`.suggest-btn` (matches existing secondary buttons; `margin-right: auto` inside
`.composer-actions` to left-align).

### 5. Backend — regression test only

`src-tauri/src/export.rs` tests: one new case proving the fence is emitted verbatim (guards anyone
"helpfully" escaping bodies later). No production Rust changes; `build_publish_payload` already has
verbatim-body coverage in `review.rs` tests — extend an existing payload assertion with a
suggestion-fence body if trivially cheap, otherwise skip.

### Files touched

- `src/lib/diff.ts` — `rightLinesText`, `suggestionFence` (new exports)
- `src/lib/diff.test.ts` — helper tests
- `src/components/ReviewView.tsx` — `FileReview` seed computation; `LineWidget`, `Composer`,
  `CommentItem` optional props + button
- `src/components/Markdown.tsx` — `pre` override
- `src/components/Markdown.test.tsx` — render tests
- `src/styles.css` — suggestion panel + button styles
- `src-tauri/src/export.rs` — one verbatim-fence test (test code only)
- `ROADMAP.md` — drop the §1 "Suggested changes" bullet (final commit)

## Tasks

1. `diff.ts`: add `rightLinesText` + `suggestionFence` with tests in `diff.test.ts`. Buildable
   alone.
2. `Markdown.tsx`: `pre` override + `styles.css` panel rules, tests in `Markdown.test.tsx`.
   Buildable alone (rendering works even before composing exists).
3. `Composer`: `suggestionSeed` prop + button; `FileReview`: `composerSuggestionSeed`; thread
   through `LineWidget`.
4. `CommentItem`: `suggestionSeed` prop + Write-tab button (appends + triggers debounced save);
   `FileReview`: `suggestionSeedFor`; thread through `LineWidget`.
5. `export.rs`: verbatim-fence test.
6. `ROADMAP.md`: remove the shipped bullet.

## Test matrix

### Rust — `src-tauri/src/export.rs`

| Test | Asserts |
|---|---|
| `markdown_preserves_suggestion_fence_verbatim` | a comment whose body is `"fix:\n\n```suggestion\nlet x = 1;\n```"` appears character-for-character in `render_markdown` output (use the existing `detail`/`comment` fixtures) |

### vitest — `src/lib/diff.test.ts`

| Test | Asserts |
|---|---|
| `rightLinesText: single normal line` | returns `["…content…"]` without a diff-sign prefix |
| `rightLinesText: insert line` | insert changes resolve via `lineNumber` |
| `rightLinesText: range spanning insert + normal` | ordered lo→hi contents |
| `rightLinesText: line in collapsed gap` | returns `null` |
| `rightLinesText: range partially missing` | one resolvable + one missing → `null` |
| `rightLinesText: delete lines invisible` | a delete at line N does not satisfy RIGHT line N |
| `suggestionFence: basic` | exactly `` ```suggestion\n<lines>\n``` `` |
| `suggestionFence: content containing ``` ` `` | fence grows to 4 backticks |

(Build hunks with the `HunkData` literal pattern already used by `buildFullFileFile`.)

### vitest — `src/components/Markdown.test.tsx`

| Test | Asserts |
|---|---|
| `renders suggestion fences as a panel` | `.suggestion-block` present, header text "Suggested change", code content inside `.suggestion-block-new`, and **no** default `pre > code.language-suggestion` |
| `leaves other code fences alone` | ```` ```ts ```` still renders a plain `pre` |
| `renders empty suggestion as removal` | `.suggestion-block-empty` with "(removes the selected lines)" |

### vitest — `src/components/ReviewView` (Composer, exported)

| Test | Asserts |
|---|---|
| `Composer: no seed → no button` | "Insert suggestion" absent when prop omitted/null |
| `Composer: insert appends seed` | click button → textarea value is the fence; type first, then click → `text + "\n\n" + fence` |

## Gates

Standard suite — all must pass:

- `pnpm exec tsc --noEmit`
- `pnpm build`
- `pnpm test` (vitest run)
- `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`
- `cargo test --manifest-path src-tauri/Cargo.toml`

No spec-specific extra gate.

## Manual verify

1. `pnpm tauri dev`; open a local review (virtual PR).
2. Click an **added (green) RIGHT** line → composer opens with "± Insert suggestion". Click it →
   textarea contains a ```` ```suggestion ```` fence holding that line's exact current text.
3. Shift-click two lines below (range) → re-insert → fence holds all selected lines in order.
4. Click a **deleted (red) LEFT** line → no button. Open "Comment on file" → no button.
5. Save a suggestion comment, reopen it: Write tab shows the button (appends + autosaves);
   Preview tab shows the "Suggested change" panel, not a plain code block.
6. Expand a collapsed gap (local target), comment on a revealed context line → button works there
   too; on a still-collapsed line range it does not appear.
7. GitHub end-to-end: open a real PR review, add a RIGHT-side suggestion comment, publish, then
   open the PR in the browser — GitHub shows the suggestion with **Commit suggestion** enabled.
8. Export → Markdown preview: the fence appears verbatim under the comment's location heading.
9. Open a PR that has reviewer suggestions in its threads → `GithubThread` bodies show the panel.

## Out of scope

- **Applying a suggestion to the working tree** (or committing it) from the app — v1 only authors,
  renders, publishes, and exports the fence.
- Old-vs-new **mini diff rendering** in `Markdown` (needs hunk context the renderer doesn't have);
  word-level `markEdits` likewise.
- Suggestion button in `FileViewPane` / for `origin = 'file_view'` or file-level comments.
- Cursor-position insertion; "replace selection" semantics in the textarea.
- Batch-applying suggestions, GitHub's "Add suggestion to batch".
- Any backend/DB/migration change (0007/0008 stay reserved for specs 12/16).
