---
name: local-review
description: >
  Analyze a GitHub PR (or a local branch diff) in the CodeReview app. Two separate jobs:
  --explain is a reading helper (one explanatory comment per change, no review), and the
  review runs at two depths: --depth normal (Standards + Spec critique + static test
  coverage) or --depth advanced (normal + Codex adversarial review + measured coverage).
  Review findings get one severity scale, are deduplicated across layers, replace the
  previous run's findings and raise the focus tiers of the hunks they land on (a tiers-only
  focus is generated when none is fresh). With no flag it runs a review sized to the PR.
  Single layers remain available: --focus, --critique, --codex, --coverage. All analysis
  runs in fresh-context agents, never in the caller's context. Also drafts local replies to
  a PR's existing GitHub review threads. Use when asked to "review PR", "explain PR",
  "local review of PR", "advanced review", "focus review", "triage PR hunks", "filter the
  noise in PR", "critique PR", "codex review of PR", "coverage review", "how well tested is
  PR", "test coverage of PR", "review branch <name>", or "reply to PR comments/threads".
---

# Local Review Skill

Fetches a GitHub PR's diff (or a local branch diff), has fresh-context agents analyze it, and populates a
local review in the CodeReview desktop app (via the `cr` CLI) so you can browse the annotated diff in the
GUI. Everything written is tagged `ai:explain`, `ai:review`, `ai:codex` or `ai:coverage` and stays local: nothing is
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

**Explain** (a reading helper, not a review):

- `--explain` (alias `--plain`) — one `ai:explain` comment per change, at any PR size. It never touches the
  focus. A rerun replaces the previous explanations the user didn't touch. See `explain.md`.

**Review depth** (what the app's AI review menu runs):

| Depth | Layers |
|---|---|
| `--depth normal` | `--critique` (Standards + Spec) + `--coverage` in **static** mode (no test run) |
| `--depth advanced` | normal + `--codex` (adversarial) + `--coverage` in **measured** mode |

A review needs a focus so its findings can raise hidden hunks: a **fresh focus is reused**, otherwise a
**tiers-only focus** (no notes) is generated in the same run (Step 1e). A **small PR** (< 150 lines and
≤ 5 files, effective) gets no focus: nothing would be hidden, and the coverage agent tiers hunks itself. A
layer that can't run (no Codex plugin, no checkout, no runner for the measured run) is skipped and
reported; the rest continue.

**Layers** (power use, combinable with each other and with `--explain`, not with `--depth`):

- `--focus [--level focus|thorough|critical]` — tier every hunk with explanation notes (what **Generate
  focus** does in the app); see `focus.md`. Always re-plans.
- `--critique` — Standards + Spec agents, plus the Readiness agent on your own PRs; see `critique.md`
  and `readiness.md`
- `--codex[=adversarial|native] ["focus text"]` — Codex review of the checkout; see `codex.md`
- `--coverage` — how well unit and e2e tests cover the change: a 0–100 score, focus areas, a comment per
  gap and raised tiers on untested hunks; see `coverage.md`. Measured mode. Needs a checkout.
- `--with-replies` — also draft thread replies (see Reply mode). Never auto-enabled.

No `--depth` and no layer flag (with or without `--explain`): auto mode picks the review depth (Step 1b).
`--explain` alone runs only the helper, no review.

**Reply mode** (draft replies to a PR's existing GitHub review threads, GitHub PR only):

- `/local-review reply owner/repo#123`

If no arguments are provided, ask the user (AskUserQuestion) which target and layers they want.

## Prerequisites

The `cr` binary must be installed. If `cr` is not found on PATH:
```bash
cargo install --path ~/projects/codereview/src-tauri --bin cr
```

The instruction files live next to this file. Agents read them, you do not (except `codex.md` §1–3):
`explain.md`, `focus.md`, `critique.md`, `readiness.md`, `codex.md`, `coverage.md`, `merge.md`, `reply.md`. Pass the skill directory
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
gh pr view <n> -R <owner>/<repo> --json title,labels,additions,deletions,changedFiles,author,closingIssuesReferences
gh api user --jq .login
```
Branch mode: always "mine", size from `git -C <repo> diff --shortstat <base>...<head>`.
`<review-id>` target: auto picks from the review's own size (Step 1c onwards), counting it as not mine.

**1b. Auto mode** picks a review depth. Size is *effective* lines (additions + deletions) and files.

| Size | Depth |
|---|---|
| Large (> 600 lines **or** > 20 files) **and** mine | `advanced` |
| Otherwise | `normal` |

The raw gh numbers pick the depth before creation. After Step 1c, recompute the effective size without
lockfiles/generated files (it can only move `advanced` down to `normal`) and decide "small" (Step 1e) from it:
```bash
cr focus hunks RID --json | jq '[.[] | select(.rule_tier == null)] | {lines: (map(.additions + .deletions) | add // 0), files: (map(.file_path) | unique | length)}'
```
Announce one line and proceed without asking, e.g. `Auto: 820 lines / 24 files, yours → --depth advanced`.

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

**1e. Reuse or generate the focus.** Applies to a review (`--depth`, auto, or any of
`--critique`/`--codex`/`--coverage`) without an explicit `--focus`, on a non-small PR:
```bash
cr focus show RID --json | jq '{planned: (.focus.planned_at != null), stale}'   # null = no focus
```
- A planned focus with `stale: false`: **reuse** it. The merge agent raises its tiers with
  `cr focus raise` and never re-plans.
- No focus, an unplanned one, or `stale: true`: the **Focus agent** generates a **tiers-only** one (no
  notes) in the same fan-out.
- Small PR: no focus; say so.

An explicit `--focus` always launches the Focus agent **with notes**. Announce the choice, e.g.
`--depth normal: reusing the focus` or `focus is stale → tiers-only focus`.

### Step 2: Prepare the fan-out

```bash
mkdir -p /tmp/cr-RID
cr worktree RID --json        # {"path": "..."}  → WT
```
If `cr worktree` fails (no local clone), WT is empty: agents work from `cr diff RID` only and the
checkout-bound layers (`--codex`, the Standards agent's file reads) say so in their reports.

If `--codex` (or depth `advanced`): do `codex.md` §1–2 now. On a skip, write the reason to `/tmp/cr-RID/codex-skipped.txt`.

### Step 3: Fan out (one message, everything in parallel)

Send **one message** containing all of:

- `--codex` / depth `advanced` (not skipped): `codex.md` §3, a background Bash call writing `/tmp/cr-RID/codex.json`.
- `--explain`: the **Explain agent**.
- A focus to generate (Step 1e, or `--focus`): the **Focus agent**.
- `--coverage` (or depth `normal`/`advanced`): the **Coverage agent**.
- `--critique` (or any depth): the **Standards agent** and the **Spec agent**.
- `--critique` (or any depth) on a PR that is mine (author = `gh api user --jq .login`; branch mode is
  always mine): the **Readiness agent**.
- `--with-replies`: the **Reply agent**.

Every agent is `Agent` with `subagent_type: general-purpose` and a self-contained brief. Common header
for all briefs:

> Review id: RID. Checkout: WT (empty = none; use `cr diff RID`). Scratch dir: /tmp/cr-RID/. Skill
> directory: <dir>. Flags: <the resolved layer flags>. You run non-interactively: don't ask the user, don't
> open the app, don't publish anything. Finish with a report of at most 150 words.

Then per agent:

- **Explain agent**: "Read `<dir>/explain.md`." It posts its comments and reports as `explain.md` says.
- **Focus agent**: "Read `<dir>/focus.md`. Notes: none (tiers and reasons only)" (review run) or "Notes:
  yes" (explicit `--focus`). "Submit with `cr focus set RID [--level <level>] --file /tmp/cr-RID/plan.json`."
  It reports the tier counts and the summary.
- **Standards agent** / **Spec agent**: "Read `<dir>/critique.md`; you are the Standards (Spec) agent." It
  gathers its own sources, writes `/tmp/cr-RID/standards.json` (`spec.json`), and reports only the finding
  count and its worst finding.
- **Readiness agent**: "Read `<dir>/readiness.md`." It writes `/tmp/cr-RID/readiness.json` and reports
  only the finding count and its worst finding.
- **Coverage agent**: "Read `<dir>/coverage.md`." Plus: "Focus: generated in this run", "Focus: reused" or
  "Focus: none (tier hunks yourself)", and "Coverage mode: static" (depth `normal`) or
  "Coverage mode: measured" (depth `advanced`, explicit `--coverage`).
  It writes `/tmp/cr-RID/coverage.json` and reports the score and the top focus area only.
- **Reply agent**: "Read `<dir>/reply.md`." Target: `<owner> <name> <number>`.

### Step 4: Merge (only if a findings layer ran: critique, codex or coverage)

Wait for the background Codex run (you are notified; non-zero exit → write `codex failed: <first line of
stderr>` to `/tmp/cr-RID/codex-skipped.txt`) and for the agents. Then launch one fresh **Merge agent**
(common header, plus the flags and depth, whether the focus was generated, reused or is absent, and which of
`standards.json`, `spec.json`, `readiness.json`, `codex.json`, `codex-skipped.txt`, `coverage.json` exist):
"Read `<dir>/merge.md`." It normalizes severities, deduplicates across layers, prunes the previous run's
findings, posts, and raises focus tiers. It returns a report of at most 250 words.

No findings layer (`--explain` and/or `--focus` only): skip this step. Those agents already reported.

### Step 5: Open the app and report

```bash
cr open review RID
```
If it fails (older build), fall back to `open "codereview://review/RID"`.

Tell the user, from the agents' reports only: the review id and the depth or layers used (and why, if
auto); whether the focus was reused, generated or skipped (small PR); comments pruned and added per tag
(`ai:explain`, `ai:review`, `ai:codex`, `ai:coverage`); the tier counts, the hunks raised and the
`critical` hunks; with coverage, the score line and the focus areas; any layer skipped or failed and why;
that the app is open on the review; and the summary (from the agents' reports; it is not saved to the
review, so nothing is published).

Never set the review body or verdict: the body is what gets published to GitHub. This is an aid, not an approval gate.

## Reply mode: draft replies to existing GitHub threads

Composes local, unpublished replies to a PR's *existing* GitHub review-thread comments (other reviewers,
bots, etc.), not the explanatory comments. GitHub PR only; branch-diff reviews have no threads.
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
- `cr focus raise` fails because the focus went stale mid-run: report it. The finding comments are
  already posted; rerun the same depth (it regenerates the stale focus, whose agent floors tiers from the
  findings' severity, `focus.md`).
- An agent returns an error or an empty report: say which one, continue with the rest, and do not redo its
  analysis in the main context.
