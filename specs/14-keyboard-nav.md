# Spec 14 — Keyboard navigation

Implements ROADMAP §1 "Keyboard navigation — next/prev file, next/prev comment, `c` to comment on
the focused line, `j`/`k` movement" (ROADMAP.md:29-30). Frontend-only; no Rust changes.

## Problem

The review screen is mouse-only. The sole keyboard affordance in the entire app is the Escape
handler that closes the full-file pane (`src/components/FileViewPane.tsx:54-60`). Navigating a
large diff means scrolling manually or clicking rows in `FileJumpList`; finding the next comment
thread means eyeballing; starting a comment requires a precise gutter click
(`ReviewView.tsx:623-637`). There is no shortcut discoverability surface at all.

Constraints that shape the design:

- **All review tabs stay mounted** (hidden with `display:none`) so tab state survives switches
  (`src/App.tsx:52-55`, `TabPane` at `App.tsx:78-84`). A naive `window.addEventListener` in
  `ReviewView` would fire in every hidden tab at once — the existing `FileViewPane` Escape
  listener already has this latent bug.
- The "active file" concept already exists: `FileJumpList` computes `activeIndex` with a scrollspy
  (`FileJumpList.tsx:166`, effect at `:183-233`) and has a `jumpTo(index)` that scrolls + selects +
  locks the spy (`:235-247`). Keyboard file-nav must reuse it, not duplicate it.
- The per-line data needed for a `j`/`k` cursor (ordered visible diff lines, including
  user-expanded context) lives inside each `FileReview` as `metaByKey` from `indexFile`
  (`ReviewView.tsx:551-554`, `src/lib/diff.ts:74-113`) — it is not available at the `ReviewView`
  top level.

## Decisions (locked)

- **Bindings (v1):** `]` / `[` next/prev file (scroll + select in FileJumpList) · `n` / `p`
  next/prev comment thread (scroll + brief highlight flash) · `j` / `k` move a focused-line cursor
  within the **active** file's visible diff lines (visual outline) · `c` open the comment composer
  on the focused line (single-line) · `?` toggle a shortcut-help overlay · `Escape` closes
  overlay, then composer/selection, then cursor.
- **ONE window keydown listener**, in `ReviewView`, with a small dispatch table. No per-component
  listeners (FileViewPane's existing one stays as-is).
- **No zustand.** `useUIStore` is persisted (`store.ts:119-126`); ephemeral focus state doesn't
  belong there, and FileJumpList only needs to *expose* its existing `activeIndex`/`jumpTo`, not
  consume shared state. Instead: plain refs — an imperative **control ref** filled by
  `FileJumpList`, and a **registry ref** of per-file handles filled by each `FileReview`. Refs
  avoid re-rendering `ReviewView` (and its heavy diff subtree) on every scrollspy tick.
- **Active-tab gating** via `useUIStore((s) => s.activeTabId === `review-${reviewId}`)` (tab id
  shape from `store.ts:18`); the listener is only attached while this review tab is active.
- **Input guard:** every key except `Escape` is ignored when the event target is inside
  `input`, `textarea`, `select`, or `[contenteditable]`. Modifier chords
  (`ctrlKey || metaKey || altKey`) are always ignored (`?` naturally arrives as `e.key === "?"`
  with shift — do not special-case shift).
- **`n`/`p` are stateless and DOM-query based**: collect rendered thread elements inside the
  scroll container, pick the first one below / last one above the current scroll offset. Zero
  plumbing into `FileReview`; robust against expansion, viewed-collapse, and orphan blocks.
- **`j`/`k` cursor is local state in `FileReview`** (`kbFocusKey: string | null`), exposed through
  the registry handle. Cursor starts at the file's first visible line; clamps at both ends (no
  wrap, no auto-hop to the next file). `]`/`[` clamp too.
- **`c` reuses the existing selection plumbing**: it calls `setSelection` with a 1-line range on
  the focused key, which opens the existing `Composer` (autofocused textarea,
  `ReviewView.tsx:1148`) via the existing widget path (`:699-726`). No new composer code.
- **Shortcuts are inert while the file pane is open** (`filePanePath != null`) — the pane sits
  over the diff and already owns Escape.
- **Single source of truth for bindings**: one exported `BINDINGS` array drives both the dispatch
  table and the help overlay, so they cannot drift.
- Pure decision logic (editable-target check, next/prev pick math, cursor index math) lives in a
  new `src/lib/keyboard.ts` so it is unit-testable in vitest without mounting the diff.

## Design

### New module — `src/lib/keyboard.ts`

```ts
export interface Binding {
  keys: string[];        // display form, e.g. ["]"], ["n"], ["?"]
  description: string;   // e.g. "Next file"
}

/** Drives BOTH the dispatch table and the help overlay. Order = display order. */
export const BINDINGS: Binding[] = [
  { keys: ["]", "["], description: "Next / previous file" },
  { keys: ["n", "p"], description: "Next / previous comment thread" },
  { keys: ["j", "k"], description: "Move line cursor down / up (active file)" },
  { keys: ["c"], description: "Comment on the focused line" },
  { keys: ["?"], description: "Toggle this help" },
  { keys: ["Esc"], description: "Close help / composer / line cursor" },
];

/** True when keyboard shortcuts must not fire (typing in a field). */
export function isEditableTarget(target: EventTarget | null): boolean;
// impl: target instanceof Element && target.closest("input, textarea, select, [contenteditable]") != null

/** Index of the next/prev thread given element tops (scroll-space, ascending
 *  document order) and the current scroll offset. eps absorbs "already there".
 *  Returns null when there is nothing further in that direction. */
export function pickThread(tops: number[], current: number, dir: 1 | -1, eps?: number): number | null;
// dir=1: smallest i with tops[i] > current + eps; dir=-1: largest i with tops[i] < current - eps. eps default 8.

/** Next cursor key for j/k: current==null starts at keys[0]; unknown current
 *  restarts at keys[0]; otherwise clamped index move. Empty keys -> null. */
export function moveCursorKey(keys: string[], current: string | null, delta: 1 | -1): string | null;

/** Handle each FileReview registers; index = diff file index. */
export interface FileKbHandle {
  moveCursor: (delta: 1 | -1) => void;  // j/k; no-op when file is viewed/collapsed or has no lines
  openComposer: () => void;             // c; no-op without a cursor or when readOnly
  clearCursor: () => boolean;           // Escape; true if it consumed (closed selection or cleared cursor)
}

/** Handle FileJumpList publishes (refreshed every render). */
export interface JumpListHandle {
  activeIndex: number;
  fileCount: number;
  jumpTo: (index: number) => void;      // existing FileJumpList.tsx:235 behaviour
}
```

### `src/components/FileJumpList.tsx` — expose the existing nav

One new optional prop, no behaviour change:

```ts
export function FileJumpList({ reviewId, scrollRootRef, controlRef }: {
  reviewId: number;
  scrollRootRef: RefObject<HTMLElement | null>;
  controlRef?: MutableRefObject<JumpListHandle | null>;
})
```

A dep-less `useEffect` republishes `{ activeIndex, fileCount: rows.length, jumpTo }` after every
render and nulls it on unmount. `jumpTo` is the existing function (`FileJumpList.tsx:235-247`) —
keyboard file-nav therefore inherits the scrollspy lock/release behaviour for free.

### `src/components/ShortcutHelp.tsx` — NEW, small

Renders `BINDINGS` in a fixed panel. Reuse the existing modal pattern/styles
(`.modal-backdrop`/`.modal`, `src/styles.css:1389`, usage `ReviewView.tsx:1210-1217`): backdrop
click and the ✕ button call `onClose` (Escape is handled by the ReviewView dispatcher).

```ts
export function ShortcutHelp({ onClose }: { onClose: () => void })
```

```
┌──────────────────────────────────┐
│ Keyboard shortcuts            ✕ │
│ ─────────────────────────────── │
│  ] [    Next / previous file     │
│  n p    Next / previous thread   │
│  j k    Move line cursor         │
│  c      Comment on focused line  │
│  ?      Toggle this help         │
│  Esc    Close help / composer    │
└──────────────────────────────────┘   (centered, .modal-backdrop overlay)
```

Each row: `<kbd>` per key + description (`.shortcut-help-row`, `kbd` styling added to styles.css).

### `src/components/ReviewView.tsx` — one effect + plumbing

Keep the diff small; everything decision-shaped is imported from `keyboard.ts`.

**In `ReviewView` (top component, `:49`):**

```ts
const isActiveTab = useUIStore((s) => s.activeTabId === `review-${reviewId}`);
const [showHelp, setShowHelp] = useState(false);
const jumpListRef = useRef<JumpListHandle | null>(null);
const fileKbRef = useRef(new Map<number, FileKbHandle>());   // index -> handle
const cursorFileRef = useRef<number | null>(null);           // file that owns the current j/k cursor
```

The single keydown effect (attached only while `isActiveTab`; deps
`[isActiveTab, showHelp, filePanePath, reviewId]`):

```ts
useEffect(() => {
  if (!isActiveTab) return;
  const onKey = (e: KeyboardEvent) => {
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.key === "Escape") {
      if (showHelp) { setShowHelp(false); return; }
      if (filePanePath) return;                       // FileViewPane owns its own Escape
      const i = cursorFileRef.current;
      if (i != null) fileKbRef.current.get(i)?.clearCursor();
      return;
    }
    if (isEditableTarget(e.target)) return;
    if (filePanePath || showHelp) {                   // help open: only ? toggles it back off
      if (e.key === "?" && !filePanePath) setShowHelp(false);
      return;
    }
    const jl = jumpListRef.current;
    switch (e.key) {
      case "]": case "[": { /* jl.jumpTo(clamped activeIndex ± 1) */ break; }
      case "n": case "p": { /* DOM thread nav, below */ break; }
      case "j": case "k": { /* dispatch to active file, below */ break; }
      case "c": { const i = cursorFileRef.current; if (i != null) fileKbRef.current.get(i)?.openComposer(); break; }
      case "?": setShowHelp(true); break;
      default: return;
    }
    e.preventDefault();
  };
  window.addEventListener("keydown", onKey);
  return () => window.removeEventListener("keydown", onKey);
}, [isActiveTab, showHelp, filePanePath, reviewId]);
```

- **`j`/`k` dispatch:** `const i = jumpListRef.current?.activeIndex ?? 0`; if
  `cursorFileRef.current != null && cursorFileRef.current !== i`, call `clearCursor()` on the old
  file's handle first (no stale double outline); then `fileKbRef.current.get(i)?.moveCursor(±1)`
  and `cursorFileRef.current = i`.
- **`n`/`p` thread nav** (helper function in ReviewView, ~15 lines): with
  `root = diffAreaRef.current` (`:56`/`:116`):
  1. `els = [...root.querySelectorAll<HTMLElement>(".line-widget, .github-thread, .file-comments, .orphan-comments")]`
     filtered in JS to those containing a `.comment-item` or being a `.github-thread`
     (a `.line-widget` holding only an open composer is skipped; do **not** use `:has()`).
  2. `tops = els.map(el => el.getBoundingClientRect().top - root.getBoundingClientRect().top + root.scrollTop)`
     (querySelectorAll already yields document order, which is ascending scroll order).
  3. `const idx = pickThread(tops, root.scrollTop, dir)`; if null, no-op.
  4. `els[idx].scrollIntoView({ behavior: "smooth", block: "center" })`; flash:
     `el.classList.add("kb-flash")` + `setTimeout(() => el.classList.remove("kb-flash"), 900)`.

Render `{showHelp && <ShortcutHelp onClose={() => setShowHelp(false)} />}` next to the existing
`FileViewPane` conditional (`:145`), pass `controlRef={jumpListRef}` to `<FileJumpList>` (`:115`),
and thread `kbHandles={fileKbRef}` through `ReviewDiff` (`:418`) into each `FileReview`.

Also append a hint to the existing instructions line (`:118-121`): `· press ? for shortcuts`.

**In `FileReview` (`:469`):** new prop `kbHandles: MutableRefObject<Map<number, FileKbHandle>>`,
plus:

```ts
const rootRef = useRef<HTMLDivElement>(null);                 // attached to the .diff-file div (:731)
const [kbFocusKey, setKbFocusKey] = useState<string | null>(null);
const orderedKeys = useMemo(() => [...metaByKey.keys()], [metaByKey]); // indexFile walks hunks in order
```

- Reset the cursor when the diff reparses: extend the existing
  `useEffect(() => setHunks(file.hunks), [file])` (`:510`) with `setKbFocusKey(null)`.
- Registration (dep-less effect, closes over latest state):
  `useEffect(() => { kbHandles.current.set(index, { moveCursor, openComposer, clearCursor }); return () => void kbHandles.current.delete(index); });`
  - `moveCursor(d)`: if `viewed` (FileBody unrendered, `:804`) no-op; else
    `setKbFocusKey(moveCursorKey(orderedKeys, kbFocusKey, d))`.
  - `openComposer()`: if `readOnly || !kbFocusKey` no-op; else `const meta = metaByKey.get(kbFocusKey)` and
    `setSelection({ side: meta.side, anchorLine: meta.line, focusLine: meta.line, focusKey: kbFocusKey })`
    — identical shape to a plain click (`:631-636`); the existing widget loop (`:699-726`) opens
    the composer, whose textarea autofocuses (`:1148`) so subsequent letters hit the input guard.
  - `clearCursor()`: if `selection` → `setSelection(null)`, return true; else if `kbFocusKey` →
    `setKbFocusKey(null)`, return true; else false.
- Focused-row outline via `generateLineClassName`, same API `FileViewPane` already uses
  (`FileViewPane.tsx:185-198, :247`):

```ts
const generateLineClassName = useCallback(
  ({ changes, defaultGenerate }: { changes: ChangeData[]; defaultGenerate: () => string }) => {
    const base = defaultGenerate();
    if (kbFocusKey == null || !changes.some((c) => changeKeyOf(c) === kbFocusKey)) return base;
    return base ? `${base} kb-focus` : "kb-focus";
  },
  [kbFocusKey],
);
```

  Pass it through `FileBody` (one new prop, `:830`) onto `<Diff>` (`:905`). Works in both unified
  and split view (split rows carry both sides' changes).
- Scroll-follow effect: on `kbFocusKey` change,
  `rootRef.current?.querySelector(".kb-focus")?.scrollIntoView({ block: "nearest" })`. Scope the
  query to `rootRef` — never `document` — because hidden tabs render duplicate `#file-N` subtrees
  (see the scrollspy comment at `FileJumpList.tsx:177-182`).

### `src/styles.css`

```css
/* keyboard line cursor */
tr.kb-focus { outline: 2px solid var(--accent); outline-offset: -2px; }

/* n/p landing flash */
@keyframes kb-flash { from { box-shadow: 0 0 0 2px var(--accent); } to { box-shadow: none; } }
.kb-flash { animation: kb-flash 0.9s ease-out; }

/* help overlay rows + kbd chips (reuses .modal-backdrop/.modal) */
.shortcut-help-row { display: flex; gap: 12px; align-items: baseline; padding: 4px 0; }
.shortcut-help-row kbd { /* small bordered chip: monospace, 1px border, border-radius, padding */ }
```

(Exact cosmetics free; `tr.kb-focus` must visibly outline the row without changing the
selected-range colors driven by the `--diff-*-selected-*` variables at `styles.css:674-679`.)

### Data flow summary

```
window keydown (ReviewView, active tab only)
  ├─ ] [  → jumpListRef.jumpTo(activeIndex ± 1)        (FileJumpList: scroll + select + spy lock)
  ├─ n p  → DOM query in diffAreaRef → pickThread() → scrollIntoView + .kb-flash
  ├─ j k  → fileKbRef[activeIndex].moveCursor() → kbFocusKey → .kb-focus row + nearest-scroll
  ├─ c    → fileKbRef[activeIndex].openComposer() → setSelection → existing Composer
  ├─ ?    → setShowHelp(true) → <ShortcutHelp/>
  └─ Esc  → help > (file pane: defer) > selection > cursor
```

## Tasks

1. `src/lib/keyboard.ts`: `BINDINGS`, `isEditableTarget`, `pickThread`, `moveCursorKey`,
   `FileKbHandle`/`JumpListHandle` types + `src/lib/keyboard.test.ts` (builds green standalone).
2. `src/components/ShortcutHelp.tsx` + `.shortcut-help-row`/`kbd` CSS + component test.
3. `FileJumpList.tsx`: `controlRef` prop + republish effect.
4. `ReviewView.tsx`: `isActiveTab` selector, `showHelp` state, refs, the keydown effect with the
   dispatch table, `n`/`p` helper, render `ShortcutHelp`, pass `controlRef`, hint-text tweak —
   `]`/`[`/`n`/`p`/`?`/Escape fully working at this point.
5. `FileReview`/`FileBody`: `kbHandles` plumbing, `kbFocusKey` + `orderedKeys`, handle
   registration, `generateLineClassName` + `tr.kb-focus`/`.kb-flash` CSS, scroll-follow effect —
   `j`/`k`/`c` working.
6. README feature-tour blurb (one short "Keyboard shortcuts" bullet) + drop the ROADMAP §1
   keyboard-navigation item.

## Test matrix (vitest; no Rust changes)

`src/lib/keyboard.test.ts`:

| Test | Asserts |
|---|---|
| `isEditableTarget` matrix | true for `textarea`, `input`, `select`, element inside `[contenteditable]`; false for `td`, `button`, `null` |
| `pickThread` next | tops `[100, 400, 900]`, current 100 → `1` (eps skips the one we're on) |
| `pickThread` prev | current 900, dir −1 → `1` |
| `pickThread` at ends | current 900 dir 1 → `null`; current 100 dir −1 → `null`; empty tops → `null` |
| `pickThread` eps | current 396, eps 8, dir 1 → index of 900 (400 within eps is "current") |
| `moveCursorKey` start | `current=null` → `keys[0]` for both deltas |
| `moveCursorKey` clamp | at last key, delta 1 → stays last; at first, delta −1 → stays first |
| `moveCursorKey` stale/empty | unknown current → `keys[0]`; empty keys → `null` |

`src/components/ShortcutHelp.test.tsx` (testing-library, mirrors `Markdown.test.tsx` setup):

| Test | Asserts |
|---|---|
| renders every binding | each `BINDINGS[i].description` is in the document |
| close paths | clicking backdrop calls `onClose`; clicking ✕ calls `onClose`; clicking the panel body does not |

No full `ReviewView` keyboard integration test — mounting the diff stack (react-diff-view +
Tauri-invoke queries) under jsdom is out of proportion for v1; the dispatch logic is covered by
the pure-helper tests and manual verification.

## Gates

- `pnpm exec tsc --noEmit`
- `pnpm build`
- `pnpm test` (vitest run)
- `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`
- `cargo test --manifest-path src-tauri/Cargo.toml`

(The two cargo gates must still pass untouched — this spec changes no Rust.)

## Manual verify (`pnpm tauri dev`)

1. Open a local virtual-PR review with several files and at least two comments in different files.
2. `?` → help overlay lists all six rows; `Escape` (and separately backdrop click) closes it.
3. `]` repeatedly → each file scrolls to top and its row highlights in the jump list; clamps at
   the last file. `[` walks back; clamps at the first.
4. `n` → viewport jumps to the next comment thread with a brief flash; `p` returns. At the last
   thread `n` is a no-op. Verify a GitHub-PR review also stops on read-only GitHub threads.
5. `j`/`k` in the active file → an outlined row walks down/up, scrolling to stay visible,
   including through user-expanded context lines; clamps at the file's last visible line. Scroll
   another file to the top and press `j` → the cursor appears in *that* file and the old outline
   clears.
6. `c` → composer opens on the outlined line, caret in the textarea; typing `j`, `n`, `?` inserts
   characters (no navigation). `Escape` closes the composer; second `Escape` clears the outline.
   Submit a comment via `c` and confirm it lands on the focused line with `start_line` null.
7. With the review summary textarea focused, press `]`/`n`/`j` → text is typed, nothing navigates.
8. Open the full-file pane ("View file") → all shortcuts inert; `Escape` closes the pane only.
9. Open a second review tab; with tab A active, press `]` → tab B's diff position is untouched
   (switch over to confirm).
10. Open a **published** review: `]`/`[`/`n`/`p`/`j`/`k` work, `c` does nothing.

## Out of scope

- Shortcuts anywhere outside `ReviewView` (repo list, inbox, settings, tab switching) and inside
  the `FileViewPane` (it keeps only its existing Escape; no `j`/`k` there).
- Wrap-around at ends, `j` auto-advancing into the next file, vim extras (`gg`/`G`, counts),
  user-customizable bindings, a settings entry.
- Fixing `FileViewPane`'s pre-existing any-tab Escape listener (`FileViewPane.tsx:54-60`) — noted,
  not touched.
- Accessibility/roving-tabindex/focus-ring semantics beyond the visual outline; persisting cursor
  or help-overlay state.
- `n`/`p` selection memory (it is deliberately stateless scroll-relative navigation).
- Word-level `markEdits` highlighting (separate ROADMAP §1 item).
