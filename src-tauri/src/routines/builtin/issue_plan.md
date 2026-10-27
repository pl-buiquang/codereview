You are writing the implementation plan for GitHub issue {{url}} ({{title}}) in {{repo}} as an unattended routine. Your working directory is the repository's main clone: read-only for you (never edit files, commit or switch branches there). You may write only inside the notes vault.

Plan directory for repos without a tracker root: `{{plans_dir}}`.

**Guard.**
- `gh issue view {{number}} --repo {{repo}} --json title,body,labels,comments,assignees,state`
- Bail if the issue is closed, or if an open PR already references it (`gh pr list --repo {{repo}} --state open --search "{{number}} in:body"`).

**Existing plan.** `cr --json board show {{url}} | jq '[.meta[] | select(.kind=="file" and .tag=="plan")]'`.
- If one is linked, REVISE that file in place: keep its `id`, `created` and `## Comments`, treat the "Extra instructions for this run" (when present) as the revision brief, and bump `updated:`. Bail if its status is `in-progress` or `done`.
- Otherwise create a new file (next step).

**Plan path.**
- If the clone has a `CLAUDE.local.md` with a `TRACKER_ROOT:` line, use `<TRACKER_ROOT>/issues/NN-<slug>.md` (NN = next free two-digit number, `~` expanded).
- Else, if the repo's owner is the viewer (`cr` / `gh api user -q .login`), use `~/Nextcloud/Notes/projects/<repo name>/issues/NN-<slug>.md`.
- Else use `{{plans_dir}}/<repo name>/issue-{{number}}-<slug>.md` and say in your final message that the fallback directory was used.
- `mkdir -p` the directory. Use absolute paths.

**Analyse.** Read the repo's CLAUDE.md / AGENTS.md / CONTRIBUTING, GLOSSARY.md and docs/adr, the issue comments, and the code the issue points at. Grep for the other call sites of anything you would change and for the existing tests around it. Use sub-agents for wide searches.

**Write the plan** with this frontmatter and sections:

```
---
type: ticket
id: <repo name>-{{number}}
status: needs-info | ready-for-human
repo: {{repo}}
branch: pbq/issue-{{number}}
gh: {{url}}
pr:
created: <YYYY-MM-DD>
updated: <YYYY-MM-DD>
tags: [plan]
---
```

- `## Problem`: what is wrong or missing, in two or three sentences.
- `## Analysis`: current behaviour and the code paths involved, each cited as `file:line`.
- `## Approach`: the recommended approach only, no survey of alternatives.
- `## Steps`: ordered, each step commit-sized and checkable.
- `## Test seams`: where tests go (the highest seam possible) and what they assert; this is where the tdd skill applies at implementation time.
- `## Risks / side effects`: other call sites, migrations, contracts, rollout.
- `## Open questions`: each one with the default you would pick if nobody answers.
- `## Comments`: empty, kept for later notes.

Set `status: needs-info` when there are open questions, otherwise `ready-for-human`. Never set `ready-for-agent` yourself: that is the human's go-ahead after refining the plan.

**Link.** Unless the plan is already linked: `cr board meta add {{url}} --kind file --tag plan --track --label "Plan" --value <absolute plan path>`. Then `notify "Plan ready {{repo}}#{{number}}"`.

Finish with at most 5 lines: plan path, status, number of open questions.
