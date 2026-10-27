You are fixing GitHub issue {{url}} ({{title}}) in {{repo}} as an unattended routine and opening a draft PR.

**Understand the issue.**
- `gh issue view {{number}} --repo {{repo}} --json title,body,labels,comments,assignees,state`
- Read the repo's CLAUDE.md / AGENTS.md / CONTRIBUTING and the code the issue points at.
- If the issue is closed, or an open PR already references it (`gh pr list --repo {{repo}} --state open --search "{{number}} in:body"`), bail.

**Scope guard.** Continue only if the issue is SMALL: clear expected behaviour, a fix touching a few files, no product/design decision, no breaking change to an API, event, DB schema or shared contract. Otherwise bail with the reason (e.g. "spec unclear: <question>").

**Worktree.** Do the Worktree step below.

**Fix it.**
- Make the minimal change, matching the surrounding code's style. Add or update a test that covers it.
- Run the relevant tests, linters and type checks the repo uses (`uv run` in uv projects, pnpm, cargo…; `actionlint` after editing `.github/workflows/*.yml`).
- If they fail and you can't fix them, bail (no push, no PR).

**Ship.** Do the steps below.

Finish with at most 5 lines: PR URL, branch, worktree path, tests run, local review id.
