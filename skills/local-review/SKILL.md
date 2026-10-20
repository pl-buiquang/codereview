---
name: local-review
description: >
  Analyze a GitHub PR (or a local branch diff) and create a local review in the
  CodeReview app. Default: one explanatory comment per meaningful change. Optional
  layers: --focus (tier every hunk critical/review/skim/trivial so the app hides the
  noise), --critique (Standards + Spec sub-agents), --codex (Codex adversarial
  review of the PR's checkout). Also drafts local replies to a PR's existing GitHub
  review threads. Use when asked to "review PR", "explain PR", "local review of PR",
  "focus review", "triage PR hunks", "filter the noise in PR", "critique PR",
  "codex review of PR", "review branch <name>", or "reply to PR comments/threads".
---

# Local Review Skill

Fetches a GitHub PR's diff (or a local branch diff), analyzes it, and populates a
local review in the CodeReview desktop app (via the `cr` CLI) so you can browse the
annotated diff in the GUI. Everything written is tagged `ai` and stays local: nothing
is published to GitHub unless the user removes the tag or edits the comment in the app.

## Arguments

**Target** (one of):

- `owner/repo#123` · `owner repo 123` · `<github-pr-url>`
- `branch <head> [--base <base>] [--repo /path/to/repo]` — diff `<base>...<head>` (base defaults to `main`, repo to the cwd)
- `<review-id>` — an existing review (`cr show <id> --json` to check it exists)

**Layers** (all optional, combinable; none = the plain explanatory review):

- `--focus [--level focus|thorough|critical]` — tier every hunk; see `focus.md`
- `--critique` — Standards + Spec sub-agents; see `critique.md`
- `--codex[=adversarial|native] ["focus text"]` — Codex review of the checkout; see `codex.md`
- `--with-replies` — also draft thread replies (see Reply mode)

**Reply mode** (draft replies to a PR's existing GitHub review threads, GitHub PR only):

- `/local-review reply owner/repo#123`

If no arguments are provided, ask the user which target and layers they want.

## Prerequisites

The `cr` binary must be installed. If `cr` is not found on PATH:
```bash
cargo install --path ~/projects/codereview/src-tauri --bin cr
```

Read the layer files only for the flags in use. They live next to this file
(`focus.md`, `critique.md`, `codex.md`).

## Workflow

### Step 0: Validate (and register the repo, branch mode)

Branch mode: determine the repo path (`--repo`, else the cwd; confirm with
`git -C <path> rev-parse --show-toplevel`), then register it (idempotent):
```bash
cr repo add <repo-path> --json     # capture the repo id
```
Confirm both refs resolve (`git rev-parse`). Fail here, not inside parallel agents.

### Step 1: Create (or reuse) the review

```bash
cr create pr <owner> <name> <number> [--reuse-draft] --json
cr create local <repo-id> <base-ref> <head-ref> --three-dot [--reuse-draft] --json
```

Pass `--reuse-draft` when `--focus` is set, so the focus lands next to comments the user already
wrote. For a `<review-id>` target skip creation. Capture the review id `RID` (`id` field). If creation
errors with "gh: not found" or similar, tell the user to run `gh auth login`.

Read the diff once:
```bash
cr diff RID
```
If it is empty, report that and stop. If it is very large (>200 files), warn the user and ask
whether to proceed or focus on specific paths.

### Step 2: Launch the optional layers (before the main pass)

Start everything slow first, so it runs while you read the diff:

- `--codex`: follow `codex.md` §1–3 (Bash with `run_in_background: true`).
- `--critique`: follow `critique.md` §1–2 (two `Agent` calls in a single message).

They return findings in the common shape (`file, line, side, title, body, kind, ref, fix`).

### Step 3: Main pass (Claude)

**Without `--focus`: explain each change.** Parse the diff into per-file hunks. For each file with
meaningful changes, compose a clear, concise comment on **what changed and why it matters**.

- One comment per file is the default for simple changes. Otherwise explain each hunk removed/added.
- Focus on the *purpose*: "Adds retry logic for transient S3 errors" beats "Adds a try/catch block".
- Renames/moves with no logic change: one file-level comment, or skip.
- Test files: the scenario covered, not the assertions.
- Config/infra: the operational impact.
- Skip tiny related one-liners (import reordering, formatting) or group them.
- 1–3 sentences each. Dense, not verbose.

Pick the most relevant line (usually the first line of the key change) and post it:
```bash
# Line-level comment (RIGHT = new/changed lines; LEFT only for deleted code that matters)
cr comment add RID --file <path> --line <n> --side RIGHT --body "explanation..." --tag ai
# File-level comment
cr comment add-file RID --file <path> --body "explanation..." --tag ai
```
Always pass `--tag ai`: it keeps the comment local until the user removes the tag or edits it.

**With `--focus`: tier every hunk, and let the notes carry the explanation.** Follow `focus.md`
to gather and tier the hunks and build the plan, with these overrides:

- A visible hunk's (`critical`/`review`) `note` is its explanation: 1–3 sentences on what changed and
  why, then `Check: …` when there is something concrete to verify. A visible hunk with nothing worth
  saying gets no note. `skim`/`trivial` hunks get only a `reason`.
- Do not post explanation comments with `cr comment add`; `cr focus set` creates them and owns them, so a
  rerun replaces them instead of piling up.
- Do not submit the plan yet if layers are running. Merge their findings first (Step 4).
- Submit with `cr focus set RID [--level <level>] --file <plan.json>` as `focus.md` Step 4 says.

### Step 4: Merge layer findings (`--critique`, `--codex`)

Wait for the background Codex run and both critique agents. Skipped or failed layers are noted in the report.

**Anchor each finding.** Take hunks from `cr focus hunks RID --json` (it works without `--focus`
too). A hunk covers new-file lines `new_start` … `new_start + d - 1`, where `+c,d` is read from its `header`.
A finding whose `file` and `line` fall inside a hunk's range is line-anchored (side RIGHT).
Otherwise it becomes a file comment, with `L<start>-<end>` in the body when it has a range.
Findings with no `file` (a missing requirement) go only into the review body.

**Prefix the body** so each comment says where it came from, then the body and `**Fix:** …`:

- `**[codex · high]**` (the severity), `**[spec · missing]**`, `**[spec · scope-creep]**`, `**[spec · wrong]**`
- `**[standards · CLAUDE.md: <rule>]**` for documented rules, `**[smell? · Feature Envy]**` for baseline smells

**Post them:**

- Without `--focus`: `cr comment add RID … --tag ai` (or `add-file`), one per finding.
- With `--focus`: append each anchored finding to its hunk's `note` and raise that hunk's tier. Any finding
  makes it at least `review`; `codex-critical`, `codex-high` or spec `wrong` makes it `critical`. Findings
  that don't land on a hunk still go in as file comments with `cr comment add-file … --tag ai`. Then submit
  the plan. Those file comments are not owned by the focus, so a rerun does not replace them.

Keep the layers separate. Never merge or rerank findings across axes.

### Step 5: Set the review summary

2–4 sentences on the whole change (with `--focus`, reuse the plan's `summary`). With layers, append a
section per layer that ran, each with the finding count and the worst finding in that layer only:

```
## Standards   — N findings. Worst: …          (or "skipped: …")
## Spec        — N findings. Worst: …          (or "no spec available")
## Codex       — verdict needs-attention|approve. <Codex summary>
```

```bash
cr update RID --body "…" --verdict comment
```
Use `--verdict comment`. Never `approve` or `request_changes`: this is an aid, not an approval gate.

### Step 6: Open the app and report

```bash
cr open review RID
```
If it fails (older build), fall back to `open "codereview://review/RID"`.

Tell the user: the review id; comments added per layer; with `--focus`, the tier counts printed by
`cr focus set` and the `critical` hunks; any layer skipped or failed and why; that the app is open
on the review; and the summary you wrote.

## Reply mode: draft replies to existing GitHub threads

Composes local, unpublished replies to a PR's *existing* GitHub review-thread comments (other reviewers,
bots, etc.), not the explanatory comments above. GitHub PR only; branch-diff reviews have no threads.
Runs standalone (`/local-review reply owner/repo#123`) or with `--with-replies` after Step 1 using the
same `RID`.

### R1: Ensure a review exists

If not created in this run, create or reuse it:
```bash
cr create pr <owner> <name> <number> --json
```
Draft replies attach to this review but are independent of its status; they can be drafted and published
even on an already-published review.

### R2: Fetch existing GitHub threads

```bash
cr threads <owner> <name> <number> --json
```
Returns an array of threads with `id`, `isResolved`, `path`, `line`, `diffSide`, and `comments` (each with
`databaseId`, `author.login`, `body`, `createdAt`). The **root** comment is `comments[0]`. Skip resolved
threads unless the user asked to revisit them.

### R3: Decide a response per open thread

Read the full chain (including existing replies) and choose:
- **Accept** — valid; acknowledge and say what will change (or that it's already addressed).
- **Challenge** — push back with a concrete technical reason; don't be contrarian for its own sake.
- **Question** — ask for clarification when ambiguous.
- **Nit ack** — brief acknowledgment for minor/style comments.

Skip threads that need no response (a plain "LGTM"). Format: `**Accept** — <1-3 sentence rationale>`.

### R4: Create the draft reply

Use the thread root's `databaseId` and the thread's own anchor (`path`, `line`, `diffSide`). Don't recompute from the diff:
```bash
cr comment draft-reply RID \
  --thread-comment-id <root-databaseId> \
  --file <path> --line <line> --side <diffSide> \
  --body "**Accept** — rationale..."
```

### R5: Report

Tell the user how many drafts were created (and how many threads skipped/resolved); that drafts are **not
yet posted to GitHub** and must be reviewed (edit/delete) in the app, then published with "Publish N draft
replies" (or `cr comment publish-replies RID`); and open the review (`cr open review RID`).

## Error handling

- `cr` not found: show the cargo install command.
- PR missing or `gh` unauthenticated: relay the error clearly.
- Empty diff: skip commenting and report it.
- `cr repo add` fails because the path is not a git repo: tell the user.
- A layer fails (Codex missing, no spec, no checkout): continue with the others and say what was skipped.
