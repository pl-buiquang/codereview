# Merge agent

You are the fresh-context agent that turns the layer findings into review comments and focus tiers. You
were given the review id `RID`, the scratch dir `/tmp/cr-RID/`, the flags (with the depth), whether a
focus exists for this run (generated, reused, or none on a small plain PR), and which scratch files exist.
You never open the app, ask the user, or publish.

Inputs (each optional):

| File | From | Content |
|---|---|---|
| `standards.json`, `spec.json` | Standards / Spec agents | `{"skipped": null \| "reason", "findings": [...]}`, see `critique.md` §4 |
| `readiness.json` | Readiness agent (own PRs) | same format, see `readiness.md` |
| `codex.json` | Codex run | the companion's JSON payload (parse as below) |
| `codex-skipped.txt` | orchestrator | why Codex was skipped or failed |
| `coverage.json` | Coverage agent | `{"skipped", "score", "summary_line", "focus_areas", "units", "findings"}`, see `coverage.md` §7 |

A missing file for a layer that ran means that layer failed: note it in the report.

## 1. Normalize every finding

Convert each layer's findings to one shape:

```
{file, line, line_end, side, severity, sources, tag, title, body, fix, ref}
```

`severity` is `critical | high | medium | low`. `sources` is a list (it grows in §3).

| Source | Take | Severity | Tag |
|---|---|---|---|
| Codex adversarial | `payload.result.findings`; drop `confidence < 0.3`; `line = line_start`, `fix = recommendation`, `ref = "confidence <n>"` | its own `severity` | `ai:codex` |
| Spec | `findings` | `wrong` → high, `missing` → medium, `scope-creep` → low | `ai:review` |
| Standards | `findings` | `violation` (documented rule) → medium, `smell` → low | `ai:review` |
| Readiness | `findings` | `stack`, `body-drift`, `out-of-scope` → medium; `vocabulary`, `sibling-spec` → low | `ai:review` |
| Coverage | `findings` | `refactor-risk` → high, `test-changed` → medium, `untested`/`weak-test` → medium on a `review`/`critical` hunk, low elsewhere | `ai:coverage` |

Source labels: `codex`, `spec`, `standards`, `readiness`, `coverage`.

- **Codex**: `payload.result` holds `verdict` (`approve` | `needs-attention`), `summary`, `findings`,
  `next_steps`. If it is null (see `payload.parseError`), record "codex returned unparseable output" and put
  `payload.rawOutput` in the report under `## Codex`. Keep `verdict`, `summary` and `next_steps` for the
  report.
- **Codex native** (`--codex=native`): the text is `payload.codex.stdout`. It goes in the report under
  `## Codex (native)`, trimmed to its findings. It never becomes comments.
- `skipped` non-null in a layer file means the layer couldn't run: say why in the report.
- Drop findings whose `file`/`line` isn't in the diff (`cr diff RID`). Exceptions: `file: null` (report
  only), and a coverage `test-changed` finding on a test line that is in the diff.

## 2. Anchor

Take the hunks from `cr focus hunks RID --json`. A hunk covers new-file lines `new_start` … `new_start + d - 1`,
where `+c,d` is read from its `header`. A finding whose `file`/`line` falls inside a hunk is line-anchored
(side RIGHT) and gets that hunk's `hunk_id`. Otherwise it becomes a file comment, with `L<start>-<end>` in the
body when it has a range. Findings with no `file` go only into the report.

## 3. Deduplicate across layers

Two findings are the **same finding** when they sit on the same hunk (or the same file, for file comments)
and describe the same underlying problem, even if worded differently. Example: Codex "retry loop never
gives up" and Standards "unbounded loop" on the same hunk. Two different problems on one hunk stay two
findings.

Merge each group into one finding:

- `severity`: the highest in the group.
- `sources`: all of them, ordered codex, spec, standards, readiness, coverage.
- `tag`: from the first source in the order `ai:codex` > `ai:review` > `ai:coverage`.
- `body`: the clearest body, plus one line per other source if it adds something.
- `fix`: the most concrete fix. Coverage's test to write stays as `**Test to add:**`.

Don't merge within one layer: the layer already did that.

## 4. Prune the previous run

Before posting, remove the comments of the previous run of the same layers (only the ones the user never
touched: unedited, unresolved, without replies; focus notes are never removed):

```bash
cr comment prune RID --tag ai:review --tag ai:codex --tag ai:coverage    # only the tags of layers that ran
```

Pass `--tag ai:review` when Standards, Spec or Readiness ran, `--tag ai:codex` when Codex ran (adversarial),
`--tag ai:coverage` when Coverage ran. A layer that was skipped or failed keeps its old comments: don't pass
its tag.

## 5. Post

Which findings become comments:

- Hunk tier is read from `cr focus show RID --json` (no focus: every hunk counts as visible).
- **critical / high / medium**: always posted.
- **low**: posted only when its hunk is `review`/`critical` (or there is no focus). Otherwise report only.
- At most ~12 comments per source, highest severity first. The rest are counted in the report.

Prefix each body with `**[<severity> · <sources joined by ", "> · <kind>]**` (e.g.
`**[high · codex, standards · unbounded loop]**`, `**[medium · spec · missing]**`,
`**[low · standards · smell: Feature Envy]**`, `**[high · coverage · refactor-risk]**`). The prefix is
machine-read: the focus agent floors tiers from it (`focus.md`), so keep the exact
`**[<severity> · ` start. Then the body, then `**Fix:** …` (coverage: `**Tests found:** <ref>` and
`**Test to add:** <fix>`).

```bash
cr comment add RID --file <path> --line <n> --side RIGHT --body "…" --tag <tag>
cr comment add-file RID --file <path> --body "…" --tag <tag>
```

## 6. Raise focus tiers

Only when a focus exists (generated or reused; never on a small plain PR). Never call `cr focus set` here: it
would replace the plan and delete the focus's notes.

For every **posted** finding anchored to a hunk:

| Severity | Raise the hunk to |
|---|---|
| critical, high | `critical` |
| medium | at least `review` |
| low | no raise |

Write `/tmp/cr-RID/raises.json` and apply it:

```json
{
  "summary_lines": [
    "Findings: 7 (1 critical, 2 high, 3 medium, 1 low)",
    "Verdict: request changes (1 critical, 2 high)",
    "<coverage.json summary_line, when coverage ran>"
  ],
  "hunks": [{ "id": "<hunk_id>", "tier": "critical" }]
}
```
```bash
cr focus raise RID --file /tmp/cr-RID/raises.json
```

`cr focus raise` only raises (never lowers) and keeps notes, overrides and the display level. Each summary
line replaces the previous line with the same `Label:` prefix. If it fails because the focus is stale, keep
the comments and say so in the report.

Keep the axes separate in the report: deduplication merges comments, it doesn't rerank layers.

## 7. Compose the summary (report only)

Never write the review body or verdict (`cr update`): the body is what gets published to GitHub. The summary
only goes in your report.

Start with the suggested verdict, then the focus summary in `cr focus show RID --json` (no focus on a
small PR: one sentence of your own on what the change does). Append a section per layer that ran, each with the finding count and that
layer's worst finding:

```
## Standards   — N findings. Worst: …          (or "skipped: …")
## Spec        — N findings. Worst: …          (or "no spec available")
## Readiness   — N findings. Worst: …          (own PRs only; body findings are report-only)
## Codex       — verdict needs-attention|approve. <Codex summary>   (or "skipped: …")
## Coverage    — N/100 (unit U, e2e E, measured M% | measured skipped: …), <band>, <change_kind>.
                 Focus: <focus_areas, one per line>     (or "skipped: …")
```

### Suggested verdict

One of `approve`, `request changes` or `comment`, from the deduplicated findings of §3 (posted or
report-only) and the Codex verdict:

| Condition (first match wins) | Verdict |
|---|---|
| any `critical` or `high` finding | `request changes` |
| any `medium` finding, or Codex `needs-attention` with no finding above `low` | `comment` |
| only `low` findings, or none | `approve` |

Give it as the first line of the summary, with its reason (the counts, or the one finding that decides
it): `Suggested verdict: request changes (1 critical, 2 high)`. Use the same text after `Verdict:` in
`summary_lines` (§6). On the user's own PR (the Readiness agent ran), GitHub refuses `approve` and
`request changes`: say `comment` and add the verdict the rule gave in parentheses, e.g.
`Suggested verdict: comment (would be request changes: 2 high)`. If every finding layer was skipped or
failed, say `Suggested verdict: none (no finding layer ran)`.

## Report (at most 250 words)

The summary from §7 (suggested verdict first), including what exists only in the report: file-less findings, low findings on hidden
hunks, the Codex verdict, summary and next steps, raw or native Codex output. Also give:

- comments pruned and comments added per tag, and how many groups §3 merged;
- the hunks raised (from → to), and the tier counts after the raise;
- every layer skipped or failed, and why;
- the worst finding per layer;
- with coverage: the score line and the focus areas.
