---
name: board
description: >
  Attach plan/spec files, links, notes, and Claude Code session pointers to a
  card on the CodeReview app's kanban board. Use when working on an issue or
  PR tracked on the board and you want to link a plan file, a spec doc, or
  record the current session on its card — or when a SessionStart hook needs
  to detect which card the current repo/branch belongs to.
---

# Board metadata

The kanban board (Board tab in the app) tracks issues and PRs as cards moving
through Backlog → In progress/Submitted/Reviews → Done. Each card can carry
freeform metadata — plan/spec files, links, notes, and Claude Code session
pointers — shown as attachments on the card. All of this goes through the `cr`
CLI; there is no other way to write it.

If `cr` is not on PATH: `cargo install --path ~/projects/codereview/src-tauri --bin cr`.

## Resolving the card

Every `cr board` command takes either an explicit reference or `--here`:

- Explicit: a GitHub URL (`https://github.com/owner/repo/issues/42` or
  `.../pull/7`) or the short form `owner/repo#42`.
- `--here`: resolves from the current working directory — the git remote's
  `owner/repo`, then the current branch's open PR (via `gh pr view`), falling
  back to an `issue-<n>`-named branch (matching this project's own
  `pbq/issue-<n>` convention). Use this whenever you're already in the repo
  clone or worktree you're working in; it saves looking up the reference by hand.

`--here` fails if the branch has no open PR and doesn't follow the
`issue-<n>` naming convention — in that case, pass the reference explicitly or
tell the user tracking isn't possible yet.

## Attaching a plan or spec file

When you write a plan or spec file while working an issue or PR, link it to
its card so the card shows it as an attachment:

```bash
cr board meta add --here --kind file --value /abs/path/to/plan.md --tag plan
```

Use `--tag spec` for spec documents, or another short freeform tag if neither
fits — tags autocomplete in the app from ones already used, so prefer an
existing tag over inventing a near-duplicate (check `cr board meta list --here`
or ask the user if unsure). Always pass an absolute path.

If the card isn't tracked on the board yet, this command errors; add `--track`
to track it as part of the same call:

```bash
cr board meta add --here --kind file --value /abs/path/to/plan.md --tag plan --track
```

## Attaching a link or note

```bash
cr board meta add --here --kind link --value https://example.com/doc --tag design
cr board meta add --here --kind note --value "Blocked on the billing migration" --tag blocker
```

## Recording a Claude Code session

A global `SessionStart` hook (see the project's `~/.claude/CLAUDE.md` snippet)
calls this automatically when a session starts inside a tracked repo/branch —
you normally don't need to run it yourself. If you do (e.g. a long-running
session that only became associated with a card partway through):

```bash
cr board meta add --here --kind claude_session --value "$SESSION_ID" --cwd "$(pwd)"
```

Adding the same session id twice is a no-op, so this is always safe to call
without checking first. The app shows a "copy resume command" action for each
session, which runs `cd <cwd>; claude --resume <session-id>`.

## Checking what's already there

```bash
cr board show --here
cr board meta list --here
```
