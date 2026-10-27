**Worktree.** Your working directory is the repository's MAIN clone: never edit files, commit or switch branches there. All work happens in a dedicated worktree.
- `git fetch origin`
- Default branch: `gh repo view {{repo}} --json defaultBranchRef -q .defaultBranchRef.name`
- Path: `~/.codereview/worktrees/<basename of cwd>/pbq-issue-{{number}}`, branch `pbq/issue-{{number}}`.
- If that worktree exists, reuse it. Otherwise `git worktree add <path> -b pbq/issue-{{number}} origin/<default>` (if the branch already exists locally, `git worktree add <path> pbq/issue-{{number}}`).
- `cd` into it and stay there for every remaining step.
