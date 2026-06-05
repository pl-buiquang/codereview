# Spec: in-review file tree / jump list (sidebar)

## Summary / motivation

Large reviews render every changed file's diff in one long scroll (`ReviewView` → `ReviewDiff` →
one `FileReview` per file). There is no way to see the changed-file set at a glance or jump between
files. This adds a **Files** view in the **left sidebar** listing every changed file with its +/−
stats, a comment-count badge, and a viewed indicator; clicking an entry smooth-scrolls that file's
diff to the top of the main panel. It reuses data the review already loads, so it needs **no backend
change** and **no schema change**.

> **Placement note (revised).** An earlier draft put this rail *inside* `.diff-area`. We now host it
> in the **global left sidebar** as a tab, alongside the repository list. That requires restructuring
> the sidebar into a tab system — specced separately in [`sidebar-tabs.md`](sidebar-tabs.md) and
> **built first**. This spec assumes that tab system exists and describes the **Files tab** only.

## Dependencies / prerequisites

- **[`sidebar-tabs.md`](sidebar-tabs.md)** — restructure the sidebar into a tab system: a sticky
  "main" bar (app title, **+ Add repo**, **⚙ Settings**), a **Repos** tab (today's `repo-list`), and
  a per-review **Files** tab. The Files tab is the surface this spec fills. **Build it first.**

## Current state (code, accurate at authoring; re-verify)

- **Layout** — `src/App.tsx` renders `<div className="layout">` (grid `280px 1fr`, `styles.css:84`) →
  `<RepoSidebar/>` (always) + `{main}`. `main` routes on store flags: `settingsOpen → SettingsView`,
  `activeReviewId → ReviewView`, `activeRepo → RepoView`, else empty (`App.tsx:23-36`).
- **Sidebar** — `src/components/RepoSidebar.tsx`: header (`.sidebar-header`: title + "+ Add repo" +
  ⚙) over a scrollable `nav.repo-list`. No tabs yet.
- **Existing tab pattern** — `RepoView.tsx:18-61` uses `.tabs` (underline-active `<button>`s;
  `styles.css:471-489`).
- **Review data (React Query)** — in `ReviewView.tsx`: `["review", reviewId]` → `api.getReview` →
  `ReviewDetail` (carries `comments: Comment[]`, `viewed_files: string[]`); `["review-diff",
  reviewId, target.id]` → `api.reviewDiff` → raw diff text. QueryClient (`main.tsx`) has
  `retry:false`, `refetchOnWindowFocus:false` (default staleTime 0 / gcTime 5m). A sidebar-tree
  component using the **same keys** shares the cache with no extra fetch.
- **Diff parsing & helpers** — `parseDiff(diffText)` (react-diff-view) runs in `ReviewDiff`
  (`ReviewView.tsx:285`) and `DiffViewer.tsx`. `fileDisplayPath` (`lib/diff.ts:55`) and module-level
  `countChanges` (`ReviewView.tsx:806`, also duplicated in `DiffViewer.tsx`) already produce the
  label and stats.
- **FileReview** (`ReviewView.tsx:312`) — derives `path = fileDisplayPath(file)`,
  `{add,del}=countChanges(file)`, groups comments, holds **local** `viewed` state (`:333`) seeded
  from `viewed_files`, toggled at `:469-482` via `api.setFileViewed` — which **does not invalidate**
  `["review", id]`. The `.diff-file` container (`:454`) has **no id** today.
- **Store** (`src/store.ts`) — `activeRepoId`, `activeReviewId`, `settingsOpen` + actions;
  `activeRepoId`/`activeReviewId` persisted, `settingsOpen` not.

## Goals & non-goals

**Goals**
- A **Files** tab in the sidebar listing every changed file in diff order, each showing: display
  path (basename kept visible), `+add`/`−del`, a comment-count badge, and a viewed treatment
  (dim + check).
- Click a row → smooth-scroll the corresponding file's diff to the top of the main panel.
- Active-file highlight: the row for the file nearest the top of the viewport is marked active
  (IntersectionObserver).
- Keep the list in sync as comments are added/deleted and as files are marked viewed — **across the
  two component trees** (sidebar ↔ `ReviewView`).

**Non-goals (v1)**
- The tab system itself (see `sidebar-tabs.md`).
- Nested/collapsible directory tree — a **flat list** of full paths in v1.
- Keyboard navigation between files (ROADMAP §1).
- Virtualization of the list/diffs (ROADMAP §5).

## UX & behavior

- The **Files** tab is available whenever `activeReviewId` is set; empty/disabled otherwise.
  (Auto-selecting it when a review opens is a tab-system concern — see `sidebar-tabs.md`.)
- Header: "Files (N)".
- Each row: `path` (left-ellipsized so the basename stays visible), right-aligned `+add −del`, a
  comment badge when count > 0, a viewed treatment (dimmed text + check).
- Clicking a row scrolls that file to the top of the main panel; the active row highlights as you
  scroll.

## Technical design

**Frontend**
- **New component `FileJumpList`** (sidebar tree), rendered as the **Files** tab body. Independent of
  `ReviewView`; sources everything from the shared React Query cache:
  - `const reviewId = useUIStore((s) => s.activeReviewId)`.
  - `useQuery({ queryKey: ["review", reviewId], queryFn: () => api.getReview(reviewId!), enabled: reviewId != null })`
    → shares `ReviewDetail` with `ReviewView` (no refetch).
  - `useQuery({ queryKey: ["review-diff", reviewId, detail?.target.id], queryFn: () => api.reviewDiff(reviewId!), enabled: detail != null })`
    → shares diff text.
  - `const files = useMemo(() => (diff ? parseDiff(diff) : []), [diff])`.
- **Per-file rows.** For each `(file, index)`: `path = fileDisplayPath(file)`,
  `{add,del}=countChanges(file)`, `count = countByPath.get(path) ?? 0`, `viewed = viewedSet.has(path)`.
  **Export `countChanges` from `lib/diff.ts`** so `ReviewView` and the sidebar share one copy
  (currently duplicated in `ReviewView.tsx:806` and `DiffViewer.tsx`).
- **Comment counts.**
  `countByPath = useMemo(() => { const m = new Map<string,number>(); for (const c of detail.comments) m.set(c.file_path,(m.get(c.file_path)??0)+1); return m; }, [detail.comments])`.
  Counts include orphan/outdated comments (a "has discussion" signal).
- **Scroll targets (the cross-tree bridge).** Add a stable id to the diff container in `FileReview`:
  `id={`file-${index}`}` on `.diff-file` (`ReviewView.tsx:454`); pass `index` from `ReviewDiff`'s
  map. The sidebar row does
  `document.getElementById(`file-${index}`)?.scrollIntoView({ behavior: "smooth", block: "start" })`.
  **Index-based ids** (vs path-based) sidestep duplicate-display-path collisions and special-char
  concerns; both sides iterate the same `files` array so indices line up. `scrollIntoView` scrolls
  `.main-panel` (the `overflow-y:auto` container) — exactly what we want.
- **Viewed sync across trees (key change).** The sidebar reads `viewed_files` from the cached
  `ReviewDetail`, but `FileReview` keeps `viewed` in local state and `setFileViewed` doesn't
  invalidate — so they'd diverge. Fix by writing the toggle through the cache:
  - Make viewed toggling a React Query mutation: **optimistically update** the `["review", reviewId]`
    cache (`viewed_files` add/remove) and call `api.setFileViewed`; roll back + toast on error.
    (Simpler alternative: `invalidateQueries(["review", reviewId])` after success — one round-trip.)
  - Have **`FileReview` derive `viewed` from `detail.viewed_files`** (cache) instead of local
    `useState`, so the file header, the collapse behavior, and the sidebar all read one source.
  - Recommended: optimistic update (instant, no refetch); it also sets up a future "mark all viewed".
- **Active-file highlight.** In `FileJumpList`, a `useEffect` (re-run on `reviewId`/`files.length`)
  sets up an `IntersectionObserver` over the `#file-${i}` elements (cross-tree DOM access via
  `getElementById` is fine), picks the top-most intersecting one
  (`rootMargin: "0px 0px -70% 0px"`), and sets local `activeIndex` to mark the active row. Guard for
  elements not yet mounted; the observer re-attaches when the diff renders.

**Backend** — none. `set_file_viewed` already exists; `ReviewDetail` already carries everything.

**Data** — no schema change.

**CSS (`src/styles.css`)** — reuse sidebar/tab/list patterns:
- Files tab body styled like `.repo-list` (scrollable). Rows like `.repo-item` (hover, `.active` →
  `rgba(47,129,247,0.15)`); `.viewed` → `color: var(--muted)`.
- Path left-ellipsis: `direction: rtl; text-align: left; white-space: nowrap; overflow: hidden;
  text-overflow: ellipsis` (rtl truncation; acceptable for paths).
- Stats reuse `.diff-stats .add` (`#3fb950`) / `.del` (`--danger`). Comment badge as a small pill
  like `.status-badge` (`border-radius: 999px`).

## Edge cases

- **Renames/deletes:** label + scroll target key off `fileDisplayPath` / shared index — consistent
  with `FileReview`.
- **Binary files:** listed; `0/0` stats render; no badge when count 0.
- **Duplicate display paths:** ids are index-based → no scroll-target clash; comment counts keyed by
  `file_path` may merge for identical paths — acceptable v1.
- **Empty diff:** header "Files (0)", no rows.
- **No active review:** Files tab empty/disabled.
- **Switching reviews:** `reviewId` change re-queries (shared cache); observer re-attaches.
- **Outdated/orphan comments:** counted into their `file_path` badge.

## Phasing

- **v1:** Files tab — flat list, click-to-scroll, comment badges, viewed reflected from shared cache
  **+ live viewed sync** (cache write-through) **+ active-file highlight**. (Depends on
  `sidebar-tabs.md`.)
- **v2:** collapsible directory tree; "mark all viewed"; keyboard nav between files.

## Open questions

- Viewed sync: optimistic cache update (instant, recommended) vs. invalidate-and-refetch (simpler).
- Active highlight ownership: observer in `FileJumpList` (recommended) vs. a shared
  `activeFileIndex` in the UI store.
- Comment badge: count all comments (recommended, "has discussion") vs. only currently-anchored.
- Does the Files tab auto-activate when a review opens? (Tab-system concern — `sidebar-tabs.md`.)

## Acceptance criteria & verification

- With the tab system in place, opening a multi-file review and selecting **Files** lists every
  changed file with correct +/− stats and comment counts; counts update after adding/deleting a
  comment.
- Clicking a file scrolls its diff to the top of the main panel; the active row tracks scroll.
- Toggling "Viewed" in a file header dims/checks the matching Files-tab row immediately (and
  vice-versa).
- `pnpm exec tsc --noEmit` passes; `pnpm tauri dev` demonstrates the Files tab + jump behavior end to
  end on a virtual-PR review with several changed files (use `/run` or `/verify`).
