# Graph

The **Graph** tab of a repo is a GitKraken-style view of its history with light git actions. It needs a local clone. Without one the tab is disabled and the repo opens on **GitHub PRs**.

## Layout

- **Toolbar**: scope selector, context chip, last-fetch label, **Fetch** (`git fetch --prune origin`), **Pull**, **Push**, and a menu with **Reload**.
- **Sidebar** (left): collapsible **Local**, **Remote**, **Worktrees**, **Tags** and **Stashes** sections, each with a filter box. Branches and tags are shown as a folder tree, sortable by last updated or name. Click a ref to jump to its tip in the graph.
- **Graph** (center): lane-drawn commits, paged 400 at a time as you scroll. Jumping to a ref loads up to 20 pages looking for it.
- **Right panel**: the selected commit, or the `// WIP` row.
- Side panes are resizable.

## Scopes

The code has two scopes: **Local** (default, local branches) and **All** (branches, remotes and tags). There is no "Current" scope. Jumping to a remote ref while on Local shows "Switch to All to see this branch".

## Commits and diffs

- Select a commit to see subject and body, linked PRs, a copyable sha, clickable parents, author and committer, and changed files with counts.
- Click a file to preview its diff in place of the graph. `Esc` or the close button returns, and previous/next cycle files. Stashes preview the same way.
- A dashed **`// WIP +N`** row sits above HEAD when the context checkout has uncommitted changes. Its panel lists Staged, Unstaged and Untracked files.

## Git actions

Mutations always act on the **context checkout** (the main checkout or the selected worktree).

- Right-click a branch, tag, remote branch or commit, or use a row's `...` button:
  - Checkout a local branch. Checkout a remote branch as a local one. Detached checkout of a tag or commit (confirmed).
  - Create branch here (optionally checking it out). Create tag here (optionally annotated, optionally pushed). Rename branch. Delete branch. Delete tag (local only).
- Toolbar: **Pull** is `git pull --ff-only`, **Push** is `git push`. Both are disabled on a detached HEAD.
- WIP panel: stage, unstage (per file or all), discard (confirmed, not undoable), commit, **Amend**, and **Stash...** (optional message, optionally including untracked files).
- Sidebar **Stashes**: preview, apply, pop, drop (drop is confirmed).
- Sidebar **Prune gone** force-deletes local branches whose upstream is gone, after confirmation.
- Risky cases show git's message and offer an explicit escalation:
  - deleting an unmerged branch offers force delete;
  - a rejected push offers `--force-with-lease`;
  - a branch checked out in another worktree offers to switch the graph to that worktree.

## Worktrees

- Clicking a worktree makes it the graph's **context**: the HEAD marker, WIP row and actions follow it. The toolbar chip shows the context and resets it to the main checkout.
- Rows show the branch, the source (Claude, Codex, CodeReview, manual), a linked PR badge, and main/stale badges.
- Per row: **open in VS Code** (`code <path>`) and **remove** (confirmed; not offered for the main checkout). **Prune stale** appears when stale worktrees exist.
- For a branch checked out in a worktree, its menu offers **Open terminal** and **New Claude session** in that worktree, plus Focus for a live Claude session there.

## Reviewing from the graph

- Branch menu: **Review vs `<default>`** (disabled on the default branch or when none is known) and **Review vs...**, which picks any other branch as the base.
- Commit panel button: the primary action is **Review PR #n** if a PR is at that tip, else **Review vs `<default>`**, else **Review this commit**. The dropdown adds *vs...* and *This commit only* (against its first parent).
- These create a local virtual-PR review. Review vs `<default>` uses the merge-base (three-dot, GitHub semantics). In **Review vs...**, a **merge-base** checkbox toggles three-dot against plain two-dot. Its initial value comes from the `defaultThreeDot` setting (on by default).

## Freshness and watchers

- Local changes are picked up by file watchers on `.git`, held lazily and refcounted by open views. Unwatched repos reload on mount and window focus. See [ADR 0004](../adr/0004-lazy-refcounted-watchers.md).
- While the Graph is visible, a background fetch runs on open and on window focus if the auto-fetch interval (Settings, default 0 = off) has elapsed. The toolbar shows "fetched N min ago".

## Open the graph from a terminal

```bash
cr open graph                    # current repo, current branch (or exact tag if detached)
cr open graph --ref v1.2.0       # a branch, remote branch (origin/x) or tag
cr open graph --no-ref           # just the repo's Graph
cr open graph ~/projects/foo     # another repo/worktree path
cr open graph --repo 12          # registered repo id, skips detection
```

- `cr open tree` is an alias.
- A repo that is not registered yet is added automatically.
- Inside a linked worktree, that worktree becomes the graph's context.
- Under the hood it opens the deep link `codereview://repo/<repo_id>[?ref=<name>][&worktree=<path>]`. The `ref` must be a valid ref name. If it is a remote branch and the scope is Local, the graph switches to All.

## Where it lives

- `src/components/graph/`: `GraphView.tsx` (shell, scope, context), `CommitGraph.tsx` (lanes, WIP row), `RepoSidebar.tsx`, `CommitPanel.tsx`, `DiffPreview.tsx`, `GitOps.tsx` and `GitRefItems.tsx` (git actions), `BranchActionsMenu.tsx`, `CommitReviewMenu.tsx`, `ReviewVsDialog.tsx`.
- `src/lib/graphLayout.ts`: lane layout.
- `src-tauri/src/graph/`: log, refs, commit, WIP and mutation ops.
- `src-tauri/src/commands/graph.rs`, `commands/worktree.rs`: Tauri commands.
- `src-tauri/src/deep_link.rs`, `src-tauri/src/bin/cr.rs`: deep link and `cr open graph`.
