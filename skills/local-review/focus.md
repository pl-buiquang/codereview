# Focus review

Focus is a lens over a normal CodeReview review. You classify hunks; the app
hides the low tiers behind one-click bars and lists them in a digest. Nothing you
write is published to GitHub. Tiers, summary and notes are only a reading aid.

All data goes through the `cr` CLI. Always put flags such as `--json` **after**
the subcommand (`cr focus hunks 12 --json`): in app mode only commands starting
with `cr focus`, `cr diff` or `cr show` are permitted. Notes you write are added
as `ai:explain`-tagged draft comments — never published to GitHub unless the user removes
the tag or edits them.

The review already exists. Its id is `RID` (the id given to you, or the one the caller created).
Don't create or open reviews here. Don't pass `--level` to `cr focus set` unless the caller
tells you to: the user controls the display level in the app at any time, independent of generation.

If `cr` is not on PATH: `cargo install --path ~/projects/codereview/src-tauri --bin cr`.

## Step 1: Gather

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

## Step 2: Tier rubric

- `critical`: security/auth, data loss or migrations, concurrency, public
  API/contract changes, money/billing, error handling on hot paths, subtle logic changes.
- `review`: normal behavior changes, new logic, non-trivial tests of new behavior.
- `skim`: straightforward glue, small refactors, logging, docs-adjacent code,
  test updates that follow mechanically.
- `trivial`: formatting, renames/moves without logic change, imports, version
  bumps, generated output, fixtures, lockfiles.

**When in doubt, tier up.** Hiding something important is far worse than showing noise.

## Step 3: Write the plan

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
  `cr focus set` turns each one into an `ai:explain`-tagged draft comment on that hunk — you never call
  `cr comment add` yourself for this.
- List every hunk you tiered. Omitted hunks fall back to their rule tier, else `review`.
- Use only ids from Step 1. Unknown ids make the whole plan fail.

## Step 4: Submit

Write the plan JSON to a file with the Write tool, then submit it. Don't pass JSON
inline in a Bash command: Claude Code blocks commands that mix braces and quotes.

```bash
cr focus set RID --file <plan.json>
```

Add `--level <focus|thorough|critical>` only when the caller asked for a starting
display level (default `focus`).

Use any temp path (e.g. `/tmp/focus-RID.json`), unless you were given a path: in app mode
that path is the only writable location.

If it reports unknown ids, the diff changed under you. Re-run Step 1 and fix the plan.

`cr focus set` prints the tier counts. Report them, plus anything flagged `critical`.
