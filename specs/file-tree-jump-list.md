# Spec: in-review file tree / jump list

## Summary / motivation

Large reviews render every changed file's diff in one long scroll (`ReviewDiff` →
`FileReview` per file). There is no way to see the set of changed files at a glance or to jump
between them. This adds a per-review sidebar (jump list) of the changed files with their +/−
stats, a comment-count badge, and a viewed indicator; clicking an entry scrolls its diff into
view. It reuses data the review already loads, so v1 needs **no backend change**.

## Current state

- `src/components/ReviewView.tsx`
  - `ReviewView` (line ~28) renders `ReviewHeader` + a `.diff-area` div containing `ReviewDiff`.
  - `ReviewDiff` (line ~268) does `const files = useMemo(() => parseDiff(diffText), [diffText])`
    (line 285) and maps `files` → `FileReview`.
  - `FileReview` (line ~312) derives `path = fileDisplayPath(file)`, computes `{ add, del }` via
    `countChanges(file)` (line 436), groups this file's comments into `commentsByKey` (lines
    336–351), and holds local `viewed` state seeded from `detail.viewed_files` (line 333).
  - The file container is `<div className="diff-file">` (line 439) with header `.diff-file-header`
    (line 440). It has **no id**, so nothing can scroll to it yet.
- `ReviewDetail` (`src/lib/types.ts`) already carries `comments: Comment[]` (sorted by
  `(file_path, line)`) and `viewed_files: string[]`.
- `fileDisplayPath` / `countChanges` already produce the label and stats the rail needs.
- There is **no file tree / jump list today**. The only left sidebar is `RepoSidebar.tsx`, which
  lists repositories, not files.
- Styling: single `src/styles.css`; `.layout` is a `280px 1fr` grid; diff list is `.diff-files`
  (flex column, gap 16px). CSS variables for theming.

## Goals & non-goals

**Goals**
- A list of every changed file in diff order, each showing: display path, `+add`/`−del`, a
  comment-count badge, and a viewed checkmark/dim.
- Click an entry → smooth-scroll the corresponding file's diff into view.
- Keep the list in sync as comments are added/deleted and as files are marked viewed.

**Non-goals (v1)**
- Nested/collapsible directory tree — a **flat list** of full paths in v1.
- Keyboard navigation between files (tracked separately in ROADMAP §1).
- Virtualization of the list/diffs (ROADMAP §5).
- Persisting rail collapsed/expanded width beyond a simple toggle.

## UX & behavior

- A rail on the left of the diff area (collapsible), header "Files (N)".
- Each row: `path` (ellipsized from the left so the filename stays visible), right-aligned
  `+add −del`, a comment badge when count > 0, and a viewed treatment (dimmed text + check).
- Current-file highlight: the row for the file nearest the top of the viewport is marked active
  (via an `IntersectionObserver`; optional in v1 — clicking is the must-have).
- Clicking a row scrolls that file to the top of the diff area.

## Technical design

**Frontend**
- **Lift the parsed files.** Move `const files = parseDiff(diffText)` from `ReviewDiff` up so the
  rail and the diff list share one array (pass `files` into both, or compute in `ReviewView` and
  pass down). Avoids parsing twice.
- **Per-file comment counts.** Derive once in the parent:
  `const countByPath = new Map<string,number>()` by iterating `detail.comments` and bumping
  `c.file_path`. (Counts include orphan/outdated comments; that's fine for a "has discussion"
  signal. If we want only-anchored counts, compute per `FileReview` instead — call out as a minor
  choice.)
- **Scroll targets.** Give each file container a stable id: `id={`file-${path}`}` on the
  `.diff-file` div (line 439). Clicking a rail row does
  `document.getElementById(`file-${path}`)?.scrollIntoView({ behavior: "smooth", block: "start" })`.
  Use the same `fileDisplayPath` value on both sides so ids match exactly.
- **Viewed state in the rail.** Read from `detail.viewed_files` for the initial render. Note the
  current architecture keeps the authoritative live `viewed` toggle as local state **inside each
  `FileReview`** (line 333), so the rail won't see live toggles unless we lift it. Options:
  (a) v1 — rail reflects `viewed_files` and updates on the next `getReview` invalidation; or
  (b) lift `viewed` to a `Map<path, boolean>` in the parent with write-through to
  `api.setFileViewed`, and have `FileReview` consume it. Recommend (b) for live sync since it also
  enables a future "mark all viewed" action; spec leaves the choice to the implementer.
- New component, e.g. `FileJumpList`, rendered inside `.diff-area` beside `.diff-files`.

**Backend**
- None for v1. All data (`comments`, `viewed_files`, diff text) is already in `ReviewDetail`.

**Data**
- No schema change. (`comment` and `file_view_state` tables already suffice.)

**CSS (`src/styles.css`)**
- Make `.diff-area` a 2-column flex/grid: a fixed-width rail (~240px, collapsible) + the existing
  `.diff-files`. Reuse existing variables (`--bg-elev`, `--border`, `--muted`, `--accent`).
- Stats reuse `.diff-stats .add` / `.del` colors; comment badge styled like existing pills.
- Path ellipsis: `direction: rtl; text-align: left; text-overflow: ellipsis; overflow: hidden`
  (or a left-truncation utility) so the basename stays visible.

## Edge cases

- **Renames:** show `newPath` (matches `fileDisplayPath`). Optionally render `old → new`.
- **Deletes:** `fileDisplayPath` returns `oldPath`; viewed/comment keys must use that same value.
- **Binary files:** still listed (no diff body); stats may be 0/0 — show the path, no badge.
- **Duplicate display paths** (theoretically, rename collisions): ids could clash; key by
  `index`-suffixed id if a collision is detected, or accept the first match (document the choice).
- **Empty diff:** rail shows "Files (0)" / hidden; nothing to jump to.
- **Outdated/orphan comments** still belong to a `file_path` → counted in that file's badge.

## Phasing

- **v1:** flat list, click-to-scroll, comment badges, viewed reflected from `viewed_files`.
- **v1.1:** active-file highlight via `IntersectionObserver`; live viewed sync (lift state).
- **v2:** collapsible directory tree; "mark all viewed"; integrates with keyboard nav.

## Open questions

- Comment badge: count all comments on the file vs. only currently-anchored ones?
- Rail placement: left of the diff (inside `.diff-area`) vs. reuse the global left sidebar region.
- Should rail collapsed/expanded state persist (Zustand `store.ts`) or be ephemeral?

## Acceptance criteria & verification

- Opening a multi-file review shows a rail listing every changed file with correct +/− stats and
  comment counts; counts update after adding/deleting a comment.
- Clicking a file scrolls its diff to the top of the viewport.
- Files marked viewed are visually distinguished in the rail.
- `pnpm exec tsc --noEmit` passes; `pnpm tauri dev` shows the rail and jump behavior end to end on
  a virtual-PR review with several changed files (use the `/run` or `/verify` flow).
