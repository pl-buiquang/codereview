# Git graph actions

## Context
The Graph tab (`specs/archive/22-git-graph-view.md`) was read-only. This adds the git operations you reach for while reviewing, so there is no round trip to a terminal. **Light ops only**: no merge, rebase or cherry-pick UI.

## Decisions (confirmed with user)
- **Target:** every mutation runs in the graph's context checkout (main, or the selected worktree). If git refuses a checkout because another worktree holds the branch, offer to switch the graph to that worktree.
- **Safety:** run the safe form first (`branch -d`, plain `push`, `switch` that git refuses on a dirty tree). On a recognised refusal show git's message with an explicit escalation behind a confirm (`Force delete` → `-D`, `Force push` → `--force-with-lease`). No auto-stash.
- **Surface:** context menus on sidebar branch/tag/stash rows, graph ref pills and commit rows. Pull/Push in the graph toolbar. Stage/commit/stash in the WIP panel.
- **Checkout:** local branches; remote branches (creates a tracking branch); commits and tags as *detached* with a warning confirm.
- **Branches/tags:** create at any commit or ref (branch: "check out after create", default on; tag: optional message makes it annotated, "push to origin", default off). Rename and delete local branches; delete local tags. **No remote deletes.** Pushing a tag at creation is the only remote ref write.
- **Pull/Push:** current branch of the context checkout only. `pull --ff-only`; `push`, or `push -u origin <branch>` when there is no upstream; rejected push offers `--force-with-lease`. `GIT_TERMINAL_PROMPT=0` so credential prompts cannot hang the app.
- **WIP panel:** per-file stage/unstage/discard (discard confirms), stage all / unstage all, commit box with Amend (empty message + amend keeps the old message), Stash with message and include-untracked. No hunk staging.
- **Stashes:** sidebar STASHES section; click previews the diff, per-row Apply / Pop / Drop (drop confirms).
- **Terminal / Claude:** only for branches checked out somewhere; opens in that worktree (tmux window when available, like the board). The menu also lists live Claude sessions in that cwd to focus.

## Backend
`src-tauri/src/graph/ops.rs` holds pure functions over a checkout path (tested against temp repos). `commands/graph.rs` exposes them as `git_*` commands that resolve the checkout with `resolve_checkout` and run on the blocking pool. `checkout_open_claude` lives in `commands/worktree.rs` next to `worktree_open_terminal`.

Recognised refusals are tagged in the error string as `git error: [kind] message`:

| kind | trigger | UI |
|---|---|---|
| `not_fully_merged` | `branch -d` on unmerged work | confirm → `-D` |
| `checked_out_elsewhere` | branch held by another worktree | confirm → switch context |
| `dirty_tree` | checkout would overwrite local changes | error toast |
| `push_rejected` | non-fast-forward push | confirm → `--force-with-lease` |
| `not_fast_forward` | `pull --ff-only` cannot fast-forward | error toast |

## Commands
`git_checkout_branch`, `git_checkout_remote_branch`, `git_checkout_detached`, `git_create_branch`, `git_delete_branch`, `git_rename_branch`, `git_create_tag`, `git_delete_tag`, `git_stage`, `git_unstage`, `git_discard`, `git_commit`, `git_pull`, `git_push`, `git_stash_push`, `git_stash_list`, `git_stash_apply`, `git_stash_pop`, `git_stash_drop`, `git_stash_diff`, `checkout_open_claude`. All take `(repo_id, worktree_path)` first.

## Frontend
`GitOpsProvider` (`components/graph/GitOps.tsx`) wraps the Graph tab when the repo has a clone. It owns the mutations, refusal handling, query invalidation and the name/message dialog. `useGitOps()` returns `null` outside it, so menus and the WIP panel render read-only there. Menu entries live in `GitRefItems.tsx` and extend `BranchActionsMenu`.

## Out of scope
Merge / rebase / cherry-pick / reset, hunk or line staging, remote branch or tag deletion, ahead/behind counts, pull strategies other than fast-forward.
