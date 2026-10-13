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

All data goes through the `cr` CLI. Always put flags such as `--json` **after**
the subcommand (`cr focus hunks 12 --json`): in app mode only commands starting
with `cr focus`, `cr diff` or `cr show` are permitted. Notes you write are added
as `ai`-tagged draft comments — never published to GitHub unless the user removes
the tag or edits them.

## Modes

**Terminal mode** (default): the user invoked `/focus-review` with one of:

- `/focus-review https://github.com/o/r/pull/123 [--level focus|thorough|critical]`
- `/focus-review o/r#123`
- `/focus-review branch <head> [--base main] [--repo <path>]`
- `/focus-review <review-id>`

**App mode**: the system prompt says `App mode. Review id = N.` Use review N.
Skip Step 1 (resolving the review) and Step 6 (opening the app), and don't create or open any review.
Don't pass `--level` to `cr focus set` in app mode — the user controls the display
level in the app at any time, independent of generation.

If `cr` is not on PATH: `cargo install --path ~/projects/codereview/src-tauri --bin cr`.

## Step 1: Resolve the review (terminal mode only)

Reuse the latest draft review on the target, so a focus lands next to any comments the user already wrote.

- PR URL / `o/r#123`: `cr create pr <owner> <name> <number> --reuse-draft --json`
- Branch: get the repo path (`--repo`, else `git rev-parse --show-toplevel`), then:
  - `cr repo add <path> --json` and capture `id`;
  - `cr create local <repo-id> <base> <head> --three-dot --reuse-draft --json`.
- Review id: use it as-is (`cr show <id> --json` to check it exists).

Capture the review id `RID` from the `id` field.

## Step 2: Gather

```bash
cr focus hunks RID --json
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
  `cr focus set` turns each one into an `ai`-tagged draft comment on that hunk — you never call
  `cr comment add` yourself for this.
- List every hunk you tiered. Omitted hunks fall back to their rule tier, else `review`.
- Use only ids from Step 2. Unknown ids make the whole plan fail.

## Step 5: Submit

Write the plan JSON to a file with the Write tool, then submit it. Don't pass JSON
inline in a Bash command: Claude Code blocks commands that mix braces and quotes.

```bash
cr focus set RID --level <level> --file <plan.json>
```

`--level` is optional and only matters in terminal mode: it sets the review's starting
display level (default `focus`). Omit it in app mode — see Modes above.

In terminal mode use any temp path (e.g. `/tmp/focus-RID.json`). In app mode use the
path given in the app-mode instructions, which is the only writable location.

If it reports unknown ids, the diff changed under you. Re-run Step 2 and fix the plan.

## Step 6: Open (terminal mode only)

```bash
cr open review RID
```

Tell the user the tier counts printed by `cr focus set`, plus anything flagged `critical`.
