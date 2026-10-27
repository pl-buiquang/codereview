You are implementing the approved plan for GitHub issue {{url}} ({{title}}) in {{repo}} as an unattended routine, and opening a draft PR.

**Find the plan.**
- `cr --json board show {{url}} | jq '[.meta[] | select(.kind=="file" and .tag=="plan")] | last'`. Read that file.
- Bail if no plan is linked, if the file is missing, or if its frontmatter `status:` is not `ready-for-agent` (reason: "plan not approved: set status ready-for-agent").
- The issue may have changed since: re-read it (`gh issue view {{number}} --repo {{repo}} --json title,body,state,comments`). Bail if it is closed, or if an open PR already references it (`gh pr list --repo {{repo}} --state open --search "{{number}} in:body"`).

**Worktree.** Do the Worktree step below, then set the plan's `status: in-progress`, `branch: pbq/issue-{{number}}` and bump `updated:`.

**Implement.**
- Follow `## Steps` in order. Read the repo's CLAUDE.md / AGENTS.md / CONTRIBUTING first and match the surrounding code's style.
- Use the `tdd` skill at the plan's `## Test seams` (they are agreed: do not ask). Run typecheck and single test files regularly, the full relevant suite at the end, and every linter the repo uses (`uv run` in uv projects, pnpm, cargo…; `actionlint` after editing `.github/workflows/*.yml`).
- You may commit once per plan step, each with a one-line Conventional Commit header (see the Commit step below). Keep unrelated changes out.
- If the plan turns out wrong or incomplete, do the smallest correct thing, and record the deviation under `## Comments` in the plan with the date.

**Self-review.** Run the `mattpocock-skills:code-review` skill with the plan file as the spec (fixed point: the default branch's merge-base), fix every Standards and Spec finding that is real, and re-run the tests. If tests still fail and you cannot fix them, bail (no push, no PR).

**Ship.** Do the steps below. In the PR body add the line `Refs: <absolute plan path>` next to `Closes #{{number}}`.

**Close the loop.** Set the plan's `pr: <pr-url>`, bump `updated:` and add a dated entry under `## Comments` (PR opened, local review id).

Finish with at most 5 lines: PR URL, branch, worktree path, tests run, local review id.
