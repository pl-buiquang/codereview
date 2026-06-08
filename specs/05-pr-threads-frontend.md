# 05 — Existing PR threads: render inline (read-only)

**Layer:** Frontend · **Dependencies:** 01 (Markdown), 04 (`pr_review_threads`) · **Wave:** 3

> Read `00-overview.md` first (locked decisions, conventions, anchors).

## Goal

Fetch a PR's existing review threads and render them **inline on the diff**, read-only and visually
distinct from local drafts ("from GitHub"). Threads that don't line up with the current diff
(outdated, or whose line/path isn't present) fall into a distinct GitHub-orphan sub-block — reusing
the existing orphan fallback rather than re-anchoring (re-anchoring is deferred, §2).

## Prerequisites (must already exist)

- `<Markdown>` from spec 01.
- `api.prReviewThreads(owner, name, number)` + `PrThread`/`PrThreadComment`/`PrActor` types from
  spec 04.

## Anchoring contract (the crux)

`src/lib/diff.ts::indexFile` returns `keyByAnchor`, keyed `"SIDE:line"` where **RIGHT = new/head
line, LEFT = old/base line**. A GitHub thread carries `diffSide` (`LEFT`/`RIGHT`) and `line` (the
position on that side in the diff). So a thread anchors with:

```ts
const key = thread.diffSide && thread.line != null
  ? keyByAnchor.get(`${thread.diffSide}:${thread.line}`)
  : undefined;
```

- If `key` is found → render the thread as a widget on that line (next to any local comment widget).
- If not found (`isOutdated`, `line == null`, file-level, or the line isn't in the current
  `gh pr diff`) → **GitHub-orphan** for that file. `startLine` is display-only; anchor on `line`,
  exactly as local multi-line comments collapse to their end line.

## Files to touch

- **New:** `src/components/GithubThread.tsx` — read-only thread display.
- **New (or extend `src/lib/diff.ts`):** a small pure `anchorByLine` helper (see below) + its test.
- `src/components/ReviewView.tsx`:
  - Fetch threads once and thread them down to `FileReview`.
  - In `FileReview` (≈ 362): compute `threadsByKey` + `orphanThreads` for the file; merge thread
    widgets into the existing `widgets` map; pass `orphanThreads` into `FileBody`.
  - In `FileBody` (≈ 701/745): render a distinct "GitHub threads not on the current diff" sub-block.
- `src/styles.css` — `.github-thread` styling (distinct from `.line-widget`/`.comment-item`).

## Steps

1. **Pure anchoring helper** (testable; satisfies "share one grouping helper"):
   ```ts
   // lib/diff.ts
   export function anchorByLine<T>(
     items: T[],
     sideLine: (t: T) => { side: string; line: number | null } | null,
     keyByAnchor: Map<string, string>,
   ): { byKey: Map<string, T[]>; orphans: T[] } { /* ... */ }
   ```
   Use it for GitHub threads here. (Optionally refactor the existing comment grouping in `FileReview`
   /`FileViewPane` to use it too — nice, not required. If you do, keep behavior identical.)

2. **`GithubThread.tsx`** — props `{ thread: PrThread }`, fully read-only:
   - A header row with thread badges: **Resolved** (when `isResolved`), **Outdated** (when
     `isOutdated`), and a "from GitHub" marker so it's unmistakably not a local draft.
   - For each comment in `thread.comments`: a card with author avatar + `author.login`, relative
     time via `timeAgo(comment.createdAt)` (`src/lib/timeAgo.ts`), the body via
     `<Markdown source={comment.body} />`, and a "View on GitHub" link calling `api.openUrl(comment.url)`.
   - When `isResolved` (and `isCollapsed`), default to a **collapsed** summary ("Resolved · N
     comments") that expands on click. No edit/delete/reply affordances.

3. **Fetch threads** in `ReviewView` (top-level) for `github_pr` targets only:
   ```ts
   const threadsQuery = useQuery({
     queryKey: ["pr-threads", owner, name, number],
     enabled: target.kind === "github_pr" && !!owner && !!name && number != null,
     queryFn: () => api.prReviewThreads(owner, name, number),
   });
   ```
   Pass `threadsQuery.data ?? []` down through `ReviewDiff` → `FileReview`.

4. **In `FileReview`:** filter threads to this file (`thread.path === path`), then
   `anchorByLine(fileThreads, t => ({ side: t.diffSide, line: t.line }), keyByAnchor)` →
   `{ byKey: threadsByKey, orphans: orphanThreads }`.
   - **Merge into `widgets`:** the current loop builds `widgets[key] = <LineWidget .../>`. Update the
     key set to also include `threadsByKey.keys()`, and make each `widgets[key]` render **both** the
     existing local `LineWidget` (comments + composer, unchanged) **and** any
     `threadsByKey.get(key)` rendered as `<GithubThread>` — e.g. wrap them in a fragment so a line
     with both a local draft and a GitHub thread shows both. Do not let GitHub threads overwrite or
     disable local commenting.
   - Pass `orphanThreads` to `FileBody`.

5. **In `FileBody`:** below the existing local `.orphan-comments` block, add a separate
   `.github-orphan-threads` block (only when `orphanThreads.length > 0`) titled e.g. "GitHub threads
   not on the current diff" rendering each via `<GithubThread>`. Keep it **visually distinct** from
   local orphans.

6. **Styles** in `src/styles.css`: `.github-thread` (a GitHub-tinted left border / badge), the
   resolved/outdated badges, and the GitHub-orphan block. Don't restyle local widgets.

## Edge cases to handle

- A thread whose `path` matches no file in the current diff: rare for the PR's own diff, but possible
  (file outside the diff). For v1, such threads simply won't appear under any `FileReview`. If easy,
  surface a count somewhere; otherwise add a code comment noting the limitation — **don't silently
  pretend full coverage** (see overview: no silent caps).
- File-level threads (`subjectType === "FILE"` or `line == null`) → GitHub-orphan block (or the
  per-file file-comments area if you prefer; orphan block is the simpler v1).
- Local virtual-PR targets: `threadsQuery` is disabled; nothing renders. Confirm no regressions.

## Acceptance criteria

- Existing PR review threads render inline on their anchored lines, read-only, visually distinct from
  local drafts; a line can show both a local draft widget and a GitHub thread.
- Resolved threads are badged/collapsible; outdated and un-anchorable threads appear in the
  distinct GitHub-orphan sub-block.
- Thread comment bodies render as Markdown (spec 01); "View on GitHub" opens via `api.openUrl`.
- Local draft commenting (click-to-comment, ranges, edit, delete, file/file-view comments) works
  **exactly as before**.
- For local reviews, no threads are fetched or shown.
- `pnpm exec tsc --noEmit` clean.

## Verification

- `pnpm exec tsc --noEmit`; `pnpm test` — add a unit test for `anchorByLine` (anchored vs orphan,
  LEFT/RIGHT, null line) in `src/lib/diff.test.ts` style.
- Manual: `pnpm tauri dev`, open a PR that has inline review threads (resolved + active + outdated):
  confirm inline placement, distinct styling, resolved collapse, outdated→orphan block, and that
  local commenting is unaffected.

## Notes / gotchas

- Keep the threads query **separate** from `["review", id]` so comment autosave invalidations don't
  refetch GitHub threads (and vice-versa).
- Do not persist threads or convert them into `comment` rows — they are ephemeral and read-only.
- `gh pr diff` (the diff source for PR targets) is the merge-base/"Files changed" diff; outdated
  threads anchored to a superseded commit will legitimately not find a line → orphan is correct.
