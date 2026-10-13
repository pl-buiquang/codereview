---
name: focus-review
description: >
  Generate a Focus overlay for a review in the CodeReview app: classify every
  diff hunk of a GitHub PR or local branch diff into critical/review/skim/trivial,
  write a short summary and targeted "check this" notes, so the app shows only
  the hunks worth reviewing (with a digest of what was hidden and why). Use when
  asked to "focus review", "triage PR hunks", "filter the noise in PR", or
  "/focus-review <pr|branch|review-id>".
---

# Focus review

Focus is a lens over a normal CodeReview review. You classify hunks; the app
hides the low tiers behind one-click bars and lists them in a digest. Nothing you
write is published to GitHub. Tiers, summary and notes are only a reading aid.

All data goes through the `cr` CLI. You never write review comments yourself:
notes become comments only if the user clicks **Adopt** in the app.

## Modes

**Terminal mode** (default): the user invoked `/focus-review` with one of:

- `/focus-review https://github.com/o/r/pull/123 [--level focus|thorough|critical]`
- `/focus-review o/r#123`
- `/focus-review branch <head> [--base main] [--repo <path>]`
- `/focus-review <review-id>`

**App mode**: the system prompt says `App mode. Review id = N.` Use review N.
Skip Step 1 (resolving the review) and Step 6 (opening the app), and don't create or open any review.

The default level is `focus`. Pass the requested level through to `cr focus set`.

If `cr` is not on PATH: `cargo install --path ~/projects/codereview/src-tauri --bin cr`.

## Step 1: Resolve the review (terminal mode only)

Reuse the latest draft review on the target, so a focus lands next to any comments the user already wrote.

- PR URL / `o/r#123`: `cr --json create pr <owner> <name> <number> --reuse-draft`
- Branch: get the repo path (`--repo`, else `git rev-parse --show-toplevel`), then:
  - `cr --json repo add <path>` and capture `id`;
  - `cr --json create local <repo-id> <base> <head> --three-dot --reuse-draft`.
- Review id: use it as-is (`cr --json show <id>` to check it exists).

Capture the review id `RID` from the `id` field.

## Step 2: Gather

```bash
cr --json focus hunks RID
```

Each entry has `hunk_id`, `file_path`, `header`, `additions`, `deletions`,
`rule_tier`, `rule_reason` and a short `preview`. Hunks with `rule_tier` set
(lockfiles, binary, generated, snapshots, renames) are already tiered
`trivial`. Leave them out of your plan unless you disagree. An entry with an empty `header` stands for a whole file that has no textual hunks.

For every other hunk, read the full text:

```bash
cr focus hunk RID <hunk_id>
```

If the working directory is a clone of the repo, use Read/Grep/Glob to check
cross-file impact: callers of a changed function, tests covering it, config that
references it. Spend effort where the tier is uncertain. Don't re-read obvious hunks.

For very large diffs (hundreds of hunks), tier whole files from the listing
and previews first, then fetch full text only for hunks that may be `critical`
or `review`.

## Step 3: Tier rubric

- `critical`: security/auth, data loss or migrations, concurrency, public
  API/contract changes, money/billing, error handling on hot paths, subtle logic changes.
- `review`: normal behavior changes, new logic, non-trivial tests of new behavior.
- `skim`: straightforward glue, small refactors, logging, docs-adjacent code,
  test updates that follow mechanically.
- `trivial`: formatting, renames/moves without logic change, imports, version
  bumps, generated output, fixtures, lockfiles.

**When in doubt, tier up.** Hiding something important is far worse than showing noise.

## Step 4: Write the plan

```json
{
  "summary": "Adds retry with backoff to the uploader; rest is a config rename.",
  "hunks": [
    { "id": "3f9a1c0b2d4e", "tier": "critical", "note": "Retry loop has no max-elapsed bound." },
    { "id": "a01b22c3d4e5", "tier": "trivial", "reason": "mechanical rename" }
  ]
}
```

- `summary`: 2–5 sentences on what the change does and where the risk is.
- `reason`: required for every `skim`/`trivial` hunk. The digest groups hidden
  hunks by reason, so **reuse the exact same wording** for the same kind of noise
  (e.g. always `mechanical rename`, `import reordering`, `test fixture update`).
  Keep it short and lowercase, without hunk-specific details.
- `note`: only on visible tiers (`critical`/`review`), and only when it says something concrete to
  check ("`timeout` is now seconds, callers pass ms"). Skip generic praise or restating the diff. Most hunks need no note.
- List every hunk you tiered. Omitted hunks fall back to their rule tier, else `review`.
- Use only ids from Step 2. Unknown ids make the whole plan fail.

## Step 5: Submit

Pipe the plan in on stdin (this works in both modes; app mode has no file-write tool):

```bash
cr focus set RID --level <level> --file - <<'EOF'
{ "summary": "…", "hunks": [ … ] }
EOF
```

If it reports unknown ids, the diff changed under you. Re-run Step 2 and fix the plan.

## Step 6: Open (terminal mode only)

```bash
cr open review RID
```

Tell the user the tier counts printed by `cr focus set`, plus anything flagged `critical`.
