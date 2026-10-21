# GitHub integration

All GitHub access goes through the [`gh` CLI](https://cli.github.com/). The app stores no token; authentication is whatever `gh auth login` set up.

## Requirements and failure behaviour

- **Settings → Environment** shows the `gh` path and a "GitHub auth" row. If `gh` is not on `PATH` it says to install the GitHub CLI. If `gh` is found but not logged in it says to run `gh auth login`.
- A repo's **GitHub PRs** tab shows "Not authenticated with GitHub. Run `gh auth login` in a terminal, then reopen." when auth is missing, and does not fetch.
- Packaged builds recover the login-shell `PATH` at startup so Homebrew's `gh` resolves.
- Local-only features (Graph, virtual-PR reviews) need only `git`.

## Open PRs and starting a review

- A repo's **GitHub PRs** tab lists its open PRs. It refreshes according to [Auto Sync](#sync).
- **Review** on a PR creates (or reuses) the GitHub-PR target and starts a review pinned to the PR's current commits.
- In the Graph, a commit at the tip of a PR branch shows that PR in the commit panel, with "Review PR #n" in its review menu.

## GitHub threads in the diff

- Existing review threads are fetched live and drawn in the diff, marked "GitHub", with Resolved / Outdated badges. Resolved threads collapse.
- They are never stored in the review or exported. They come from the [mirror](../adr/0002-disposable-github-mirror-soft-refs.md) and live fetches.
- **Resolve / Unresolve** acts on GitHub immediately (GraphQL `resolveReviewThread` / `unresolveReviewThread`) and needs push access to the repo.
- **Reply...** offers two paths when the thread is anchored in the current diff:
  - *Reply now* posts to GitHub immediately (REST reply on the thread's root comment).
  - *Add draft reply* stores a local draft under the thread. Drafts can be edited, deleted and published one by one, or all at once from the header ("Publish N draft replies").
- On a locked (pending or published) review the Reply button is hidden. Resolve/Unresolve is still shown.

## PR header and actions

- The review header shows state, CI, review-decision and auto-merge chips. The collapsible PR meta panel shows mergeability, +/- and file counts, reviewers, labels, checks and the description.
- The draft toggle, branch update and auto-merge live in the **Review actions** menu (`ReviewView.tsx`), not in the meta panel, which is read-only:
  - **Sync now** refreshes the PR and checks for new commits. The review stays on its pinned commits; **Update review** (draft reviews only) moves the pin and re-anchors comments.
  - **Enable / Disable auto-merge** runs `gh pr merge --auto --squash --delete-branch` (or `--disable-auto`). Disabled for closed PRs and for draft PRs.
  - **Convert to draft / Mark ready for review** runs `gh pr ready [--undo]`. Disabled when the PR is not open.
  - **Update branch (merge / rebase)** runs `gh pr update-branch`. Enabled only when the PR is open, `BEHIND`, and you can update the branch.

## Publishing

Only reviews on a GitHub PR can be published. Local virtual-PR reviews are export-only.

- **Publish as draft** (the primary button) stages the review on GitHub as a **PENDING** review (the `event` is omitted), visible only to you. Locally the review becomes `published_pending` and is locked.
  - **Submit review** sends the pending review with the stored verdict and locks it as published.
  - **Discard pending review** deletes it on GitHub and unlocks the local draft.
  - A pending review cannot be deleted locally until it is discarded.
  - GitHub allows one pending review per reviewer per PR; a conflict is reported with a clear message.
- **Publish now** (dropdown) posts immediately as Comment, Approve or Request changes, after a confirmation. It cannot be undone.
- A Comment review with no comments and no summary is rejected. Approve and Request changes may be empty.
- Comments are re-anchored to the review's pin before publishing. After a one-shot publish the app makes a best-effort capture of GitHub's comment ids.
- Published reviews are locked: no edits, no second publish.

## Clean up closed PRs

On the global **Reviews** list, **Clean up** looks up the state of every PR-backed review (one batched query) and asks before deleting the reviews of MERGED or CLOSED PRs, drafts included. Reviews pending on GitHub are excluded. A review that cannot be deleted is skipped rather than aborting the batch.

## Sync

- **Settings → Sync → Auto Sync**: Off, On focus, or Periodic. Neighbouring settings: how often to look for new PRs/issues, and the Board's auto-merge monitor interval.
- A Rust sync engine owns scheduling. Views only send hints, and updates arrive as `mirror-updated` events. See [ADR 0003](../adr/0003-backend-owned-sync-engine.md).
- GitHub facts live in a disposable local mirror that holds no user state, and Settings can wipe and rebuild it. See [ADR 0002](../adr/0002-disposable-github-mirror-soft-refs.md).
- The open Review tab's PR meta and threads, and the repo PR list, are still per-tab queries gated by the same Auto Sync setting.

## Known limits

- Pending reviews are pushed as a whole draft. There is no incremental comment-by-comment sync onto an existing pending review.
- A pending review created on github.com is not adopted.
- Draft toggle, auto-merge and update-branch failures surface `gh`'s error text.

## Where it lives

- `src-tauri/src/gh.rs`: `gh` wrappers (threads, reply/resolve, pending review, draft, auto-merge, update branch).
- `src-tauri/src/commands/gh.rs`: PR list, meta, thread and PR-action commands.
- `src-tauri/src/commands/review.rs`: `publish_review`, `publish_review_pending`, `submit_pending_review`, `discard_pending_review`, `find_closed_pr_reviews`, `delete_reviews`, draft replies.
- `src/components/RepoView.tsx`: GitHub PRs tab.
- `src/components/GithubThread.tsx`, `PublishButton.tsx`, `PrMetaPanel.tsx`, `ReviewView.tsx` (header and actions menu), `ReviewsView.tsx` (Clean up), `SettingsView.tsx`.
- `src-tauri/src/sync/`, `src-tauri/src/mirror/`: sync engine and mirror.
