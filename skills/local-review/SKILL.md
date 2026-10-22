---
name: local-review
description: >
  Analyze a GitHub PR (or a local branch diff) and create a local review in the
  CodeReview app. With no layer flags it auto-detects the mode from the PR's size and
  author: small PR = one explanatory comment per change; bigger PR = --focus (tier every
  hunk so the app hides the noise); big PR of yours = --focus + --codex (Codex adversarial
  review), plus --critique when it closes an issue. Explicit layers: --focus, --critique
  (Standards + Spec sub-agents), --codex, --plain. All analysis runs in fresh-context
  agents, never in the caller's context. Also drafts local replies to a PR's existing
  GitHub review threads. Use when asked to "review PR", "explain PR", "local review of PR",
  "focus review", "triage PR hunks", "filter the noise in PR", "critique PR",
  "codex review of PR", "review branch <name>", or "reply to PR comments/threads".
---

# Local Review Skill

Fetches a GitHub PR's diff (or a local branch diff), has fresh-context agents analyze it, and populates a
local review in the CodeReview desktop app (via the `cr` CLI) so you can browse the annotated diff in the
GUI. Everything written is tagged `ai:explain`, `ai:review` or `ai:codex` and stays local: nothing is
published to GitHub unless the user removes the tag or edits the comment in the app.

## You are an orchestrator

This skill often runs right after the caller wrote the PR. The analysis must not share the caller's
context, so **every piece of analysis runs in a fresh `general-purpose` agent** that gets only a
self-contained brief. In the main context:

- Never read the diff, the hunks or the findings (no `cr diff`, no `cr focus hunk`, no full
  `cr focus hunks`). Only counts and metadata, via `jq`.
- Never write comments, plans or the summary yourself. Agents do, and return a short report.
- Do the setup, the fan-out and the final report. Nothing else.

Subagents cannot spawn subagents, so all fan-out happens from here.

## Arguments

**Target** (one of):

- `owner/repo#123` · `owner repo 123` · `<github-pr-url>`
- `branch <head> [--base <base>] [--repo /path/to/repo]` — diff `<base>...<head>` (base defaults to `main`, repo to the cwd)
- `<review-id>` — an existing review (`cr show <id> --json` to check it exists)

**Layers** (all optional, combinable). None given = auto mode (Step 1b):

- `--plain` — force the plain explanatory review (no auto, no layers)
- `--focus [--level focus|thorough|critical]` — tier every hunk; see `focus.md`
- `--critique` — Standards + Spec agents; see `critique.md`
- `--codex[=adversarial|native] ["focus text"]` — Codex review of the checkout; see `codex.md`
- `--with-replies` — also draft thread replies (see Reply mode). Never auto-enabled.

Any of `--plain`/`--focus`/`--critique`/`--codex` disables auto: use exactly what was given.

**Reply mode** (draft replies to a PR's existing GitHub review threads, GitHub PR only):

- `/local-review reply owner/repo#123`

If no arguments are provided, ask the user (AskUserQuestion) which target and layers they want.

## Prerequisites

The `cr` binary must be installed. If `cr` is not found on PATH:
```bash
cargo install --path ~/projects/codereview/src-tauri --bin cr
```

The instruction files live next to this file. Agents read them, you do not (except `codex.md` §1–3):
`explain.md`, `focus.md`, `critique.md`, `codex.md`, `merge.md`, `reply.md`. Pass the skill directory
(the directory of this SKILL.md) in every brief.

## Workflow

### Step 0: Validate (and register the repo, branch mode)

Branch mode: determine the repo path (`--repo`, else the cwd; confirm with
`git -C <path> rev-parse --show-toplevel`), then register it (idempotent):
```bash
cr repo add <repo-path> --json     # capture the repo id
```
Confirm both refs resolve (`git rev-parse`). Fail here, not inside parallel agents.

### Step 1: Pick the mode, create (or reuse) the review

**1a. Metadata (auto mode only, before creating).**
```bash
gh pr view <n> -R <owner>/<repo> --json additions,deletions,changedFiles,author,closingIssuesReferences
gh api user --jq .login
```
Branch mode: always "mine", no linked issue, size from `git -C <repo> diff --shortstat <base>...<head>`.
`<review-id>` target: skip auto (plain, unless layers were given).

**1b. Auto mode** (no layer flag). Size is *effective* lines (additions + deletions) and files.

| Size | Layers |
|---|---|
| Small: < 150 lines **and** ≤ 5 files | plain explain |
| Medium | `--focus` |
| Large: > 600 lines **or** > 20 files | `--focus`; add `--codex` (adversarial) if the PR is mine; add `--critique` if it has a closing issue |

The raw gh numbers pick the tier before creation. After Step 1c, recompute
the effective size without lockfiles/generated files and only ever **lower** the tier:
```bash
cr focus hunks RID --json | jq '[.[] | select(.rule_tier == null)] | {lines: (map(.additions + .deletions) | add // 0), files: (map(.file_path) | unique | length)}'
```
Announce one line and proceed without asking, e.g. `Auto: 420 lines / 14 files, yours → --focus --codex`.

**1c. Create.**
```bash
cr create pr <owner> <name> <number> --json
cr create local <repo-id> <base-ref> <head-ref> --three-dot --json
```
`cr create` reuses the target's unpublished review if there is one (so the analysis lands next to comments
the user already wrote) and creates one only when none exists or all are published. For a `<review-id>`
target skip creation. Capture the review id `RID` (`id` field). If
creation errors with "gh: not found" or similar, tell the user to run `gh auth login`.

**1d. Size guard.**
```bash
cr focus hunks RID --json | jq '[.[].file_path] | unique | length'
```
`0`: report an empty diff and stop. `> 200`: AskUserQuestion whether to proceed or limit to specific paths
(the only prompt left in the auto path).

### Step 2: Prepare the fan-out

```bash
mkdir -p /tmp/cr-RID
cr worktree RID --json        # {"path": "..."}  → WT
```
If `cr worktree` fails (no local clone), WT is empty: agents work from `cr diff RID` only and the
checkout-bound layers (`--codex`, the Standards agent's file reads) say so in their reports.

If `--codex`: do `codex.md` §1–2 now. On a skip, write the reason to `/tmp/cr-RID/codex-skipped.txt`.

### Step 3: Fan out (one message, everything in parallel)

Send **one message** containing all of:

- `--codex` (not skipped): `codex.md` §3, a background Bash call writing `/tmp/cr-RID/codex.json`.
- The **Explain agent** (always).
- `--critique`: the **Standards agent** and the **Spec agent**.
- `--with-replies`: the **Reply agent**.

Every agent is `Agent` with `subagent_type: general-purpose` and a self-contained brief. Common header
for all briefs:

> Review id: RID. Checkout: WT (empty = none; use `cr diff RID`). Scratch dir: /tmp/cr-RID/. Skill
> directory: <dir>. Flags: <the resolved layer flags>. You run non-interactively: don't ask the user, don't
> open the app, don't publish anything. Finish with a report of at most 150 words.

Then per agent:

- **Explain agent**: "Read `<dir>/explain.md`" (and `<dir>/focus.md` if `--focus`). "Layers running: <list or
  none>." It writes and reports as `explain.md` says.
- **Standards agent** / **Spec agent**: "Read `<dir>/critique.md`; you are the Standards (Spec) agent." It
  gathers its own sources, writes `/tmp/cr-RID/standards.json` (`spec.json`), and reports only the finding
  count and its worst finding.
- **Reply agent**: "Read `<dir>/reply.md`." Target: `<owner> <name> <number>`.

### Step 4: Merge (only if a layer ran)

Wait for the background Codex run (you are notified; non-zero exit → write `codex failed: <first line of
stderr>` to `/tmp/cr-RID/codex-skipped.txt`) and for the agents. Then launch one fresh **Merge agent**
(common header, plus the flags and which of `plan.json`, `summary.md`, `standards.json`, `spec.json`,
`codex.json`, `codex-skipped.txt` exist): "Read `<dir>/merge.md`." It returns a report of at most 150 words.

No layer: skip this step. The Explain agent already set the summary (and submitted the plan with `--focus`).

### Step 5: Open the app and report

```bash
cr open review RID
```
If it fails (older build), fall back to `open "codereview://review/RID"`.

Tell the user, from the agents' reports only: the review id and the mode used (and why, if auto); comments
added per tag (`ai:explain`, `ai:review`, `ai:codex`); with `--focus`, the tier counts and the `critical`
hunks; any layer skipped or failed and why; that the app is open on the review; and the summary written.

Verdict is always `comment`. Never `approve` or `request_changes`: this is an aid, not an approval gate.

## Reply mode: draft replies to existing GitHub threads

Composes local, unpublished replies to a PR's *existing* GitHub review-thread comments (other reviewers,
bots, etc.), not the explanatory comments above. GitHub PR only; branch-diff reviews have no threads.
Runs standalone (`/local-review reply owner/repo#123`) or with `--with-replies` in the Step 3 fan-out
(same `RID`).

1. **Ensure a review exists** (skip if created in this run):
   ```bash
   cr create pr <owner> <name> <number> --json
   ```
   Draft replies attach to this review but are independent of its status; they can be drafted and
   published even on an already-published review.
2. **Launch one fresh Reply agent** (common header + target): "Read `<dir>/reply.md`." It does the
   fetch, decide and draft steps and reports counts.
3. **Report**: how many drafts were created (and how many threads skipped/resolved); that drafts are **not
   yet posted to GitHub** and must be reviewed (edit/delete) in the app, then published with "Publish N
   draft replies" (or `cr comment publish-replies RID`); then `cr open review RID`.

## Error handling

- `cr` not found: show the cargo install command.
- PR missing or `gh` unauthenticated: relay the error clearly.
- Empty diff: skip commenting and report it.
- `cr repo add` fails because the path is not a git repo: tell the user.
- A layer fails (Codex missing, no spec, no checkout): continue with the others and say what was skipped.
- An agent returns an error or an empty report: say which one, continue with the rest, and do not redo its
  analysis in the main context.
