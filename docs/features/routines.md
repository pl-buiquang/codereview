# Routines

A routine is a named, unattended `claude -p` run with its own prompt, scoped to a PR, a review, an
issue or the board. Launch one from a card's **Routines…** menu; runs show up in Sessions →
Routines, and routines are edited in Sessions → Config. Built-in routines can be edited and
**Reset** to their shipped text.

## Issue to PR: plan, refine, implement

Three built-ins turn an issue into a draft PR, with a plan file you review in between.

| Routine | Context | What it does |
|---|---|---|
| **Plan issue** (`issue_plan`) | issue | Reads the issue and the code (read-only, in the repo's main clone), writes a plan file and links it to the card with the `plan` tag. |
| *Agent session* (card menu) | issue | Opens Claude in the main clone with the card's notes, files and plan in its system prompt, to refine the plan with you. |
| **Implement plan** (`issue_implement`) | issue | Needs the plan's `status: ready-for-agent`. Creates the `pbq/issue-<n>` worktree, follows the plan with `tdd`, runs `code-review`, opens a draft PR (`Closes #n`, `Refs: <plan>`), then `/local-review`. |
| **Draft PR (small issue)** (`address-small-issue`) | issue | The one-shot version for small issues: no plan, a scope guard instead. |

### The plan file

A ticket in the notes vault's tracker format: frontmatter (`type`,
`id`, `status`, `repo`, `branch`, `gh`, `pr`, `created`, `updated`, `tags`) and the sections
Problem, Analysis, Approach, Steps, Test seams, Risks / side effects, Open questions, Comments.

- Location: `<TRACKER_ROOT>/issues/NN-slug.md` when the repo's `CLAUDE.local.md` sets `TRACKER_ROOT`;
  `~/Nextcloud/Notes/projects/<repo>/issues/` for your own repos; else
  `<Plans directory>/<repo>/issue-<n>-slug.md`.
- Link: a `file` item tagged `plan` on the issue card (`cr board meta add <url> --kind file --tag plan`).
  Nothing is posted to GitHub. PR cards inherit it from their issue.
- Status: Plan writes `needs-info` (open questions) or `ready-for-human`. You set `ready-for-agent`
  when you are happy; Implement refuses anything else, then moves it to `in-progress` and fills `pr:`.
- Running **Plan issue** again revises the same file; the run's note is the revision brief.

### Agent session

Issue kebab menu → **Agent session**. Claude starts in the repo's main clone (it must be linked under
Repositories) with `--append-system-prompt-file` set to the *Issue agent session* prompt (Settings →
Prompts), rendered on every open from the card's metadata. The card's latest session is resumed when
its transcript exists. Live issue sessions are not listed for focusing yet; opening again starts a
new session.

### Settings (Sessions → Config)

- **Plans directory**: fallback location for plans, available to routines as `{{plans_dir}}`.
- **Notes root**: the only tree outside the clone that **Plan issue** may write to.

## Prompt placeholders

`{{repo}} {{number}} {{url}} {{title}} {{head_ref}} {{base_ref}} {{worktree}} {{review_id}} {{plans_dir}}`.
Unknown placeholders are left as written. Handler routines may add more.
