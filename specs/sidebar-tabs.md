# Spec: sidebar tab system

## Summary / motivation

The left sidebar (`RepoSidebar`) today shows one thing — the repository list — with a header holding
the app title, **+ Add repo**, and **⚙ Settings**. To host additional per-context panels (starting
with the per-review **Files** jump list — see [`file-tree-jump-list.md`](file-tree-jump-list.md)),
restructure the sidebar into a **tab system**:

- a **sticky "main" bar** (always visible): app title, **+ Add repo**, **⚙ Settings**;
- a **Repos** tab — today's repository list;
- a per-review **Files** tab — the changed-file jump list, available when a review is open.

This is the **prerequisite** for the sidebar file jump list and should be built first.
*(Stub — to be fleshed out into a full implementation plan.)*

## Current state (code; re-verify)

- `src/App.tsx`: `.layout` grid (`280px 1fr`) = `<RepoSidebar/>` + `{main}`.
- `src/components/RepoSidebar.tsx`: `.sidebar` → `.sidebar-header` (title, "+ Add repo" `btn-primary`,
  ⚙ `btn-icon` → `openSettings`) over scrollable `nav.repo-list` (repo items, active state, remove).
  Reads `useUIStore` (`activeRepoId`, `setActiveRepo`, `openSettings`); add/remove repo via mutations
  on `["repositories"]`.
- **Existing tab pattern**: `RepoView.tsx:18-61` + `.tabs` CSS (`styles.css:471-489`) —
  underline-active `<button>`s. Reuse it.
- Store (`src/store.ts`): `activeRepoId`, `activeReviewId`, `settingsOpen`.

## Goals & non-goals

**Goals**
- A sticky main bar (title + Add repo + Settings) that stays put regardless of active tab.
- Two tabs: **Repos** (existing list) and **Files** (per-review jump list).
- Sensible default tab: **Files** auto-selected when a review opens, **Repos** otherwise; user can
  switch manually.
- Reuse the existing `.tabs` visual pattern.

**Non-goals**
- The Files tab's internals (see `file-tree-jump-list.md`).
- Nested trees, keyboard nav, drag-reorder.
- Persisting the selected tab across app restarts (ephemeral is fine; revisit later).

## UX & behavior

- `.sidebar` becomes `[sticky main bar]` + `[tab strip]` + `[active tab body]` (the body scrolls).
- Tab strip shows **Repos** always; **Files** enabled only when `activeReviewId != null`
  (disabled/hidden otherwise).
- Selecting a repo or closing a review returns focus to a sensible tab.

## Technical design (sketch)

- New `Sidebar` shell (wrap/rename `RepoSidebar`):
  - sticky main bar: app title, "+ Add repo" (existing `addRepo` mutation), ⚙ Settings
    (`openSettings`).
  - tab state: ephemeral `useState<"repos" | "files">`, OR a non-persisted `sidebarTab` in
    `useUIStore`. Auto-set to `"files"` when `activeReviewId` becomes non-null; fall back to `"repos"`
    when no review.
  - tab bodies: `<RepoList/>` (extract today's `nav.repo-list`) and `<FileJumpList/>` (from
    `file-tree-jump-list.md`).
- No backend, no schema change.

**CSS** — reuse `.tabs` / `.tabs button.active`; keep the main bar sticky; tab body scrolls
(`overflow-y:auto`) like `.repo-list`.

## Open questions

- Default-tab logic: switch to Files on review open and back to Repos on `closeReview`? Remember the
  last manual choice during a session?
- Should the Files tab show a count badge (e.g. "Files 7") in the tab strip?
- Persist the selected tab in the store (`partialize`) or keep ephemeral?

## Phasing

- **v1:** sticky main bar + Repos/Files tabs + default-tab logic; Repos tab = today's list moved
  verbatim.
- **Then:** build the Files tab per `file-tree-jump-list.md`.

## Acceptance criteria & verification

- Sidebar shows the sticky main bar (working Add repo + Settings) and Repos/Files tabs; Repos behaves
  exactly as before; Files appears for an open review.
- `pnpm exec tsc --noEmit` passes; `pnpm tauri dev` shows the restructured sidebar with no regression
  to repo add/select/remove or settings.
