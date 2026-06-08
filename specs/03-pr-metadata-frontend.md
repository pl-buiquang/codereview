# 03 — PR metadata: header panel

**Layer:** Frontend · **Dependencies:** 01 (Markdown), 02 (`pr_meta`) · **Wave:** 2

> Read `00-overview.md` first (locked decisions, conventions, anchors).

## Goal

Show the PR's metadata in the review header for GitHub-PR reviews: description, labels, CI/check
status, mergeability, review decision + per-reviewer approvals, change counts, and draft state.
Hidden entirely for local virtual-PR reviews.

## Prerequisites (must already exist)

- `<Markdown>` from spec 01 (`src/components/Markdown.tsx`).
- `api.prMeta(owner, name, number)` + the `PrMeta`/`PrActor`/`PrLabel`/`PrReviewer`/`PrCheck` types
  from spec 02 (`src/lib/api.ts`, `src/lib/types.ts`).

## Files to touch

- **New:** `src/components/PrMetaPanel.tsx`.
- `src/components/ReviewView.tsx` — render `<PrMetaPanel>` inside `ReviewHeader` (≈ line 143).
- `src/styles.css` — panel styles.

## Steps

1. **Create `PrMetaPanel.tsx`:**
   - Props: `{ owner: string; name: string; number: number }`.
   - `const q = useQuery({ queryKey: ["pr-meta", owner, name, number], queryFn: () => api.prMeta(owner, name, number) })`.
   - Loading → a compact skeleton/"Loading PR details…"; error → a muted inline error (don't blow up
     the header).
   - On success render, roughly top-to-bottom:
     - **Description:** `<Markdown source={meta.body} />` inside a **collapsible** block (default
       collapsed if long — e.g. clamp height with a "Show more" toggle). If `body` is empty, omit.
     - **Labels:** chips colored from `label.color` (hex without `#`; set `background` and pick a
       readable text color — a simple luminance check is fine).
     - **Change counts:** `+{additions} −{deletions} · {changedFiles} files`.
     - **State badges:** draft badge when `isDraft`; PR state (OPEN/MERGED/CLOSED). Reuse
       `StatusPill` from `src/components/InboxBadges.tsx` if it fits, else a small local badge.
     - **CI:** reuse `CiBadge` from `InboxBadges.tsx` for the rollup (`meta.ciState`), and optionally
       an expandable per-check list (`meta.checks`: name + state, linking to `url` via `api.openUrl`).
     - **Mergeability:** map `meta.mergeable` → `MERGEABLE` = "Mergeable" (ok), `CONFLICTING` =
       "Conflicts" (warn), `UNKNOWN`/null = "Checking…" (neutral, **not** an error).
     - **Reviews:** `reviewDecision` summary (Approved / Changes requested / Review required) plus
       per-reviewer avatars from `meta.reviews` (avatar + login + state; color by state).

2. **Render in `ReviewHeader`** (`ReviewView.tsx` ≈ 143): only when `target.kind === "github_pr"`
   and `detail.remote_owner`, `detail.remote_name`, `target.github_pr_number` are all present. Place
   it below the `review-header-top` row and above/beside the `review-summary` block. The existing
   `prUrl`/`OpenPrButton` logic already computes the owner/name/number guard — reuse it.

3. **Styles** in `src/styles.css`: a `.pr-meta-panel` with label chips, the collapsible description,
   and the reviewer/check rows. Match the existing header's visual language; mirror inbox badge
   styles for consistency.

## Acceptance criteria

- For a GitHub PR review the panel renders description/labels/CI/mergeability/approvals/counts.
- For a **local** review the panel does not render at all.
- `mergeable === "UNKNOWN"` (or null) shows a neutral "Checking…", never an error state.
- Loading and error states are handled inline without breaking the header layout.
- The PR description renders as Markdown (via spec 01), links open via `api.openUrl`.
- `pnpm exec tsc --noEmit` clean.

## Verification

- `pnpm exec tsc --noEmit`; `pnpm test` (existing tests stay green; add a light `PrMetaPanel` render
  test with a mocked `api.prMeta` if convenient — mirror `RepositoriesView.test.tsx`).
- Manual: `pnpm tauri dev`, open a real PR review → header shows metadata; open a local review →
  no panel.

## Notes / gotchas

- Keep the query **lazy and cached** by the `["pr-meta", owner, name, number]` key so re-renders
  don't refetch. Don't couple it to the `["review", id]` query.
- Don't fetch metadata for local targets (no owner/name guaranteed).
