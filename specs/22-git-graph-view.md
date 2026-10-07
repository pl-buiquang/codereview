# Spec 22 — Git graph view (replaces the Virtual PR tab)

Replaces the **Virtual PR** tab in the repo view with a GitKraken-style **Graph** tab: a sidebar of
branches/worktrees/tags, a center commit graph, a right commit panel with changed files, and a
diff preview that opens when a file is clicked.

## Problem

`src/components/RepoView.tsx` currently has three local tabs: `branches` (`BranchCompare` — two
`<select>`s, a merge-base checkbox, and a "Preview diff" / "Start review" flow), `prs`
(`PrList`), and `worktrees` (`WorktreeList`/`WorktreeRow`). There is no visual history: no commit
graph, no way to browse a branch's log, no way to inspect an arbitrary past commit's diff without
starting a review. `src-tauri/src/git.rs` has no `git log`/graph support at all — only
`list_branches`, `rev_parse`, `merge_base`, `show_file`, `diff`, `diff_shas_path`.

This spec replaces `BranchCompare` and `WorktreeList` with a single `Graph` tab that gives a
read-only view of history (committed + working directory), while keeping "start a review between
two refs" as a lightweight action reachable from a branch.

Out of scope for v1 (do not build): undo/redo/pull/push/stash/terminal toolbar actions, avatars,
AI commit actions ("Explain commit", "Recompose commit"), PR/issue/team sidebar sections, the
Path/Tree file-list toggle, blame/history-per-file, and any write operations beyond the existing
worktree remove/prune.

## Decisions (locked)

### Graph scope

A toolbar dropdown: **All / Local / Current**.
- `All` = `--branches --remotes --tags HEAD`
- `Local` = `--branches HEAD`
- `Current` = `HEAD` of the context checkout only

Paged at 400 commits per page with infinite scroll (`skip`/`limit`).

### Context checkout

The graph's "current HEAD" is **selectable**: it defaults to `repo.local_path`, but clicking a
worktree row in the sidebar makes that worktree the context. The context affects `Current` scope,
the HEAD marker on the graph, and the WIP row. A toolbar chip shows the active context with a
"reset to main checkout" action.

### WIP row (read-only)

When the context checkout has uncommitted changes, a dashed row renders above `HEAD`, labeled
`// WIP +N` (N = total changed/untracked file count). Hidden when the tree is clean. Selecting it
shows three groups in the right panel — **Staged**, **Unstaged**, **Untracked** — each a flat file
list. Clicking a file opens a diff preview:
- staged file → `git diff --cached -- <path>`
- unstaged tracked file → `git diff -- <path>`
- untracked file → diff against `/dev/null` (`--no-index`)

No commit/stage/discard actions in v1.

### Sidebar

Collapsible sections, each with a count and inline filter box: **LOCAL**, **REMOTE**,
**WORKTREES**, **TAGS**. Clicking a branch/tag row **jumps to its tip and selects it**: the graph
pages forward (capped at 20 pages) until the sha is found, then scrolls to it. If the ref's commit
isn't reachable in the current scope (e.g. a remote branch while scope = `Local`), show a toast
("Switch to All to see this branch") instead of silently failing.

Worktree rows keep today's actions — ported from `WorktreeRow`
(`src/components/RepoView.tsx:430-576`): open in VS Code (`api.openInVscode`), remove
(`api.removeWorktree`, with confirm), prune (`api.pruneWorktrees`) — and additionally set the
sidebar worktree as the graph context on click.

### Start review (branch context action)

Right-click (or a `⋯` button) on a sidebar branch row or a graph ref pill opens a small menu:
- **"Review vs `<repo.default_branch>`"** — calls `api.createReview({ repoId, baseRef: default_branch, headRef: <branch>, threeDot: true })` directly.
- **"Review vs…"** — opens a small dialog (`ReviewVsDialog`) with a base `<select>` (same source as
  today's `api.listBranches`) and a merge-base checkbox defaulting to `useSettingsStore`'s
  `defaultThreeDot`. Confirms to the same `createReview` call.

Either path then invalidates `["reviews", repo.id]` and calls `openReview(id)` — identical to the
current `BranchCompare` "Start review" behavior.

### Graph rows

Three regions per row, left to right: a ref-pill column (styled per kind: local / remote / tag /
worktree / HEAD), the SVG graph lane (computed by the layout algorithm below), and the subject
with a faint one-line body preview. No author, date, or sha columns in the row itself (available
in the right panel on selection).

### Right commit panel

- Subject + full body.
- Author (name, email, date) and committer (name, email, date) — separate lines.
- Short sha (clickable to copy) and parent sha(s) (clickable to jump to that commit in the graph).
- Aggregate A/M/D/R counts.
- A **flat** changed-file list with a status icon per `ChangedFile.status`.
- Diffing rule: a merge commit diffs against its **first parent**; a root commit (no parents)
  diffs against the empty tree; renames are detected with `-M`.

### Diff preview

Clicking a file in the right panel (or in the WIP groups) opens a preview that **replaces the
graph area** — the graph list stays mounted (just hidden) so its scroll position survives when the
preview closes. The preview header has: the file path, a split/unified toggle (default from
`useSettingsStore().defaultViewType`), prev/next-file buttons (cycling the current commit's file
list), and an `X` close button; `Esc` also closes it. It renders the diff text with the existing
`DiffViewer` component (`src/components/DiffViewer.tsx`) — no new diff-rendering code.

### Tabs in `RepoView.tsx`

The repo-local tab set (`src/components/RepoView.tsx:23`, currently
`"branches" | "prs" | "worktrees"`) becomes **`"graph" | "prs"`**:
- `graph` replaces `branches` as the default when `repo.local_path` is set (same disabled-without-
  local-clone behavior and tooltip as today).
- `prs` is unchanged.
- The `worktrees` tab is removed — its content moves into the sidebar (§ Sidebar above).
- The `.repo-reviews` side list (`RepoView.tsx:87-133`) is **hidden** on the `graph` tab (to give
  the 3-pane layout room) and continues to render on the `prs` tab, unchanged.
- `BranchCompare` and `WorktreeList`/`WorktreeRow` are deleted from `RepoView.tsx` once their logic
  has moved to the new components.

## Contract (frozen so tracks can build in parallel)

### Rust types — new `src-tauri/src/graph/mod.rs`, all `#[derive(Serialize)]`, mirrored in `src/lib/types.ts`

```rust
pub struct GraphCommit {
    pub sha: String,
    pub parents: Vec<String>,
    pub subject: String,
    pub body_preview: String, // first line of body after subject, "" if none
    pub author_name: String,
    pub author_time: i64,     // unix seconds
    pub committer_time: i64,
}

pub struct RefInfo {
    pub name: String,            // short name, e.g. "main", "origin/main", "v1.2.0"
    pub kind: RefKind,           // "local" | "remote" | "tag"
    pub sha: String,             // peeled commit sha for annotated tags
    pub is_head: bool,
}

pub struct CommitDetail {
    pub sha: String,
    pub parents: Vec<String>,
    pub subject: String,
    pub body: String,
    pub author_name: String,
    pub author_email: String,
    pub author_time: i64,
    pub committer_name: String,
    pub committer_email: String,
    pub committer_time: i64,
    pub files: Vec<ChangedFile>,
}

pub struct ChangedFile {
    pub path: String,
    pub old_path: Option<String>,  // Some(..) only for renames/copies
    pub status: ChangeStatus,      // "A" | "M" | "D" | "R" | "C" | "T"
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
}

pub struct WipStatus {
    pub head_sha: String,
    pub staged: Vec<ChangedFile>,
    pub unstaged: Vec<ChangedFile>,
    pub untracked: Vec<String>,
}
```

### Tauri commands — new `src-tauri/src/commands/graph.rs`, registered in `lib.rs`, wrapped in `src/lib/api.ts`

| Command | Signature | Implementation |
|---|---|---|
| `graph_log` | `(repo_id, scope: "all"\|"local"\|"current", worktree_path: Option<String>, skip: u32, limit: u32) -> Vec<GraphCommit>` | `git log --topo-order --format=%H%x1f%P%x1f%an%x1f%at%x1f%ct%x1f%s%x1f%b%x1e --skip --max-count`, scope flags per § Graph scope |
| `list_refs` | `(repo_id, worktree_path: Option<String>) -> Vec<RefInfo>` | `for-each-ref --format=%(refname)%09%(objectname)%09%(*objectname)%09%(HEAD) refs/heads refs/remotes refs/tags` |
| `commit_detail` | `(repo_id, sha) -> CommitDetail` | `git show -s --format=…` for the header, plus `git diff-tree -r -M --name-status --numstat [--root | <first_parent> <sha>]` for `files` |
| `commit_file_diff` | `(repo_id, sha, path, old_path: Option<String>) -> String` | diff `<first_parent or empty-tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904>..<sha>` limited to `-- [old_path] path` |
| `worktree_status` | `(repo_id, worktree_path: Option<String>) -> WipStatus` | `git status --porcelain=v1 -z`, grouped into staged/unstaged/untracked |
| `worktree_file_diff` | `(repo_id, worktree_path: Option<String>, path, kind: "staged"\|"unstaged"\|"untracked") -> String` | `git diff --cached -- path` / `git diff -- path` / `git diff --no-index -- /dev/null path` |

Safety rules (apply to every command above):
- Every `sha` argument passes through `git::validate_commit_ref` before use.
- Every path argument is passed after a literal `--` separator, never interpolated into flags.
- `worktree_path`, when `Some`, must match one of `worktree::list_worktrees(repo)`'s entries for
  that `repo_id`; otherwise return `AppError::Other("unknown worktree path")`.
- Reuse `commands/worktree.rs::repo_local_path` for resolving the repo's main checkout — make it
  `pub(crate)` (currently private). Add a small `resolve_checkout(db, repo_id, worktree_path) ->
  AppResult<PathBuf>` helper (in `commands/graph.rs` or `worktree.rs`) that returns
  `repo_local_path` when `worktree_path` is `None`, else the validated worktree path.
- `--no-index` exits with status `1` when there are differences (not an error). `worktree_file_diff`
  needs its own small process runner for that one case instead of reusing `git::run_git` (which
  treats any non-zero exit as `AppError::Git`) — put it in the new `graph/wip.rs`, do not change
  `git.rs`'s existing error behavior.

### Frontend graph-layout — new `src/lib/graphLayout.ts`, pure, no React/Tauri imports

```ts
export interface GraphRow { sha: string; lane: number; color: number; edgesUp: Edge[]; edgesDown: Edge[] }
export interface Edge { fromLane: number; toLane: number; color: number }
export function layoutGraph(commits: GraphCommit[]): { rows: GraphRow[]; maxLanes: number }
```

Lane assignment walks the topo-ordered `commits` once: track which lanes are "expecting" which
sha next; when a commit is reached, merge every lane expecting it into the first such lane (that
becomes this row's lane/color); the first parent inherits that lane; additional parents each take
an existing free lane or open a new one. Parents not yet loaded (end of the current page) keep
their lane open, running off the bottom of the rendered rows. Re-run (memoized by input identity)
whenever a new page is appended, not on every scroll tick.

## Files to touch

| File | Change |
|---|---|
| NEW `src-tauri/src/graph/mod.rs`, `log.rs`, `commit.rs`, `wip.rs` | Types + the six command implementations above |
| NEW `src-tauri/src/commands/graph.rs` | Tauri command wrappers around `graph::*`, calling `resolve_checkout` |
| `src-tauri/src/commands/worktree.rs` | Make `repo_local_path` `pub(crate)`; add `resolve_checkout` if not placed in `graph.rs` |
| `src-tauri/src/lib.rs` | `mod graph;` (alphabetical) + register the 6 new commands in `generate_handler!` |
| `src/lib/types.ts` | Add `GraphCommit`, `RefInfo`, `ChangedFile`, `CommitDetail`, `WipStatus` |
| `src/lib/api.ts` | Add `graphLog`, `listRefs`, `commitDetail`, `commitFileDiff`, `worktreeStatus`, `worktreeFileDiff` |
| NEW `src/lib/graphLayout.ts` (+`.test.ts`) | Lane layout algorithm above |
| NEW `src/components/graph/CommitGraph.tsx` (+css, +test) | Windowed graph list: ref-pill column, SVG lanes from `layoutGraph`, WIP row, selection, `scrollToSha`, `onEndReached`, `onRefContextMenu` |
| NEW `src/components/graph/RepoSidebar.tsx`, `BranchActionsMenu.tsx`, `ReviewVsDialog.tsx` (+css, +test) | Sidebar sections, filter, jump-to, worktree actions (ported from `WorktreeRow`), "Review vs…" flow |
| NEW `src/components/graph/CommitPanel.tsx`, `DiffPreview.tsx` (+css, +test) | Commit header + flat file list + WIP groups; diff preview wrapping `DiffViewer` with prev/next + close |
| NEW `src/components/graph/GraphView.tsx` (+css) | Composes the three panels, owns scope/context/selection state, the 3-pane resizable layout (pattern from `src/components/ReviewView.tsx:159`) |
| `src/components/RepoView.tsx` | Swap `branches`→`graph` tab to render `GraphView`; delete `worktrees` tab, `BranchCompare`, `WorktreeList`/`WorktreeRow`; hide `.repo-reviews` on the `graph` tab |
| `src/components/RepoView.test.tsx` | Update for the new tab set |
| `README.md`, `ROADMAP.md` | Document the Graph tab; remove any Virtual-PR-tab mentions |

## Tasks (small, file-exclusive tracks for parallel agents)

**T0 — Contract scaffold (serial, do first; everything else branches from this).**
Create `src-tauri/src/graph/{mod,log,commit,wip}.rs` with the types above and stub command bodies
returning `Err(AppError::Other("not implemented"))`; create `src-tauri/src/commands/graph.rs` and
register the 6 commands in `lib.rs`; make `repo_local_path` `pub(crate)` and add
`resolve_checkout`; add the `types.ts` and `api.ts` entries (typed, calling into the stub
commands). Gate: `tsc --noEmit`, `cargo clippy`.

**Wave 1 — parallel, one agent per row, touching only the listed files:**

| Track | Files owned | Work |
|---|---|---|
| T1 — Rust log + refs | `graph/log.rs` | Implement `graph_log` (3 scopes, skip/limit, `\x1f`/`\x1e` parse) and `list_refs` (peeled tags, `is_head`). Rust tests in fixture repos: linear history, a merge, a tag, paging across a boundary. |
| T2 — Rust commit detail | `graph/commit.rs` | Implement `commit_detail` and `commit_file_diff` (merge/root/rename cases). Rust tests for each case. |
| T3 — Rust WIP status | `graph/wip.rs` | Implement `worktree_status` and `worktree_file_diff`, including the exit-code-1-tolerant `--no-index` runner. Rust tests: staged, unstaged, untracked, renamed-staged. |
| T4 — Graph layout | `src/lib/graphLayout.ts` + `.test.ts` | Lane algorithm + a small deterministic color palette. Tests: linear chain, branch+merge, octopus merge, commits whose parents aren't loaded yet (page boundary), stability when a new page is appended. |
| T5 — Graph table component | `src/components/graph/CommitGraph.tsx` + css + test | Windowed row list (fixed row height) consuming `GraphRow[]` from T4; ref-pill column; WIP row; selection; `scrollToSha` imperative handle; `onEndReached`; `onRefContextMenu`. Build/test against fixture `GraphRow[]` data, independent of the real backend. |
| T6 — Sidebar | `src/components/graph/RepoSidebar.tsx`, `BranchActionsMenu.tsx`, `ReviewVsDialog.tsx` + css + test | Collapsible LOCAL/REMOTE/WORKTREES/TAGS sections with counts + filter; jump-to-ref callback prop (no graph coupling — parent wires it); worktree row actions ported from `WorktreeRow` (`RepoView.tsx:430-576`); "Review vs" menu + dialog calling `api.createReview`. |
| T7 — Commit panel + diff preview | `src/components/graph/CommitPanel.tsx`, `DiffPreview.tsx` + css + test | Commit header fields, flat file list with status icons, WIP grouped file lists; `DiffPreview` fetching `commitFileDiff`/`worktreeFileDiff` and wrapping `DiffViewer` with header controls + prev/next + close. |

Each track's `.css` file is separate (no shared `styles.css` edits) and only uses variables from
`src/styles/tokens.css`, so Wave 1 tracks cannot conflict with each other.

**Wave 2 — serial, after Wave 1 merges:**

- **T8 — Composition + tab swap.** Owns `src/components/graph/GraphView.tsx` (+css),
  `src/components/RepoView.tsx`, `src/components/RepoView.test.tsx`, `README.md`, `ROADMAP.md`.
  Wires T4–T7 together: scope/context/selection state, `useInfiniteQuery(["graph", repoId, scope,
  ctx])`, `["refs", repoId, ctx]`, `["wip", repoId, ctx]` (refetch on window focus),
  `["worktrees", repoId]`; a toolbar (scope select, context chip, refresh); the 3-pane resizable
  layout (pattern from `ReviewView.tsx:159`); jump-to-tip paging (cap 20 pages). Then edits
  `RepoView.tsx` per § Tabs above and deletes `BranchCompare`/`WorktreeList`/`WorktreeRow`.
- **T9 — Manual verification.** Run the checklist below against a real repo with multiple
  branches/worktrees/tags (e.g. `cardiolib` or `taboulet`).

Gate suite for every track (per `specs/ORCHESTRATION.md` §7):
```
pnpm exec tsc --noEmit
pnpm build
pnpm test
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```
Commits: single-line conventional-commit headers, no body, no co-author trailer.

## Verification

Automated: the gate suite above, plus the Rust fixture-repo tests (T1–T3) and Vitest suites
(T4–T8).

Manual (`pnpm tauri dev` against a repo with real history, e.g. `cardiolib`/`taboulet`):
1. The Graph tab is the default tab and renders lanes correctly across merges.
2. Scrolling down loads further pages without jank.
3. The All/Local/Current scope toggle changes the visible commit set correctly.
4. Clicking a remote branch while scope = Local shows the "switch to All" toast.
5. Clicking a worktree row switches the graph context and the WIP row.
6. Selecting a commit then a file opens the diff preview; closing it restores the graph's scroll
   position.
7. Merge, root, and renamed-file commits all diff correctly in the preview.
8. "Review vs `<default_branch>`" from a branch's context menu opens a new review tab.
9. A repository with no local clone shows only the GitHub PRs tab (Graph tab absent/disabled as
   today).
