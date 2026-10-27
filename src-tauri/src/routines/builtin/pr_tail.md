**Commit and push.** Commit what is left with a one-line Conventional Commit header `<type>(<scope>): <description>`, ≤70 chars, no body, no trailer. `git push -u origin pbq/issue-{{number}}`. Never `--force`, never push to the default branch.

**Draft PR linked to the issue.**
- `gh pr create --draft --repo {{repo}} --base <default> --head pbq/issue-{{number}} --title "<commit header>" --body-file <file>`
- Follow `.github/PULL_REQUEST_TEMPLATE.md` when present, else fill Summary / Evidence with the `pr` skill. The body MUST contain a line `Closes #{{number}}`: that line links the PR to the issue on GitHub. Add a short "Automated by the CodeReview routine" line plus what you tested. End the body with: 🤖 Generated with [Claude Code](https://claude.com/claude-code)
- This applies even if the repo's CLAUDE.md says to push straight to main: routines always use a branch and a draft PR.

**Board.**
- `cr board meta add {{url}} --kind note --tag automated --track --value "Draft PR <pr-url> (routine)"`
- `ISSUE_CARD=$(cr --json board show {{url}} | jq -r '.id')` then `cr board link "$ISSUE_CARD" <pr-url>` (skip the link if it fails: the "Closes" keyword already links them on GitHub).

**Local review.** Run `/local-review <pr-url>` (headless; run Codex in the foreground if it gets used).

**Notify.** `notify "Draft PR ready {{repo}}#{{number}}: <pr-url>"`.

Never merge, never mark the PR ready, never comment on the issue or the PR, never touch other branches.
