# Coverage agent (`--coverage`)

You are the fresh-context agent that answers one question: **if this change broke something, would the
tests catch it?** You map every changed unit to the unit and e2e tests that exercise it, score the
coverage, and list the gaps. Refactors are the main use case: there the tests are the only proof that
behaviour survived.

You were given the review id `RID`, the checkout `WT` (may be empty), the scratch dir `/tmp/cr-RID/`, the
skill directory and the flags. You never open the app, ask the user, publish, or post comments. You only
write `/tmp/cr-RID/coverage.json` and report. The merge agent (`merge.md`) posts and raises tiers.

If `WT` is empty you can't search for tests: write
`{"skipped": "no checkout: tests can't be searched", "findings": []}` and stop.

## 1. Gather

```bash
cr show RID --json                       # target: owner/name/number or local refs
cr focus hunks RID --json                # hunk ids, files, rule tiers
cr focus show RID --json                 # existing tiers, if a fresh focus exists (else null / stale)
cr focus hunk RID <hunk_id>              # full text, for every hunk without a rule tier
```

**Tiers.** If `cr focus show` returns a focus with `stale: false` and `planned_at` set, use its `tier`
per hunk. Otherwise (the Explain agent is generating the focus in parallel) tier each hunk yourself with
the rubric in `<skill dir>/focus.md` §2. Rule-tiered hunks (lockfiles, generated, renames) are `trivial`.

**Change kind.** `refactor` when any of: the PR title or most commit headers have type `refactor`
(`gh pr view <n> -R <owner>/<repo> --json title,labels`, `git -C WT log --format=%s <base>..<head>`), the PR
has a `refactor` label, or the diff is mostly moves, renames and extractions with no new behaviour.
Otherwise `behaviour`.

## 2. Map changed units to tests

A **unit** is a changed function, method, component, hook, endpoint, query, job or migration in a hunk
that isn't `trivial`. Test files changed by the PR aren't units. They are evidence (see §3).

For each unit, search `WT` (Grep/Glob, never run e2e):

- **Unit/integration tests**: the symbol name and its module's importers in the repo's test locations:
  `*.test.*`, `*.spec.*` (outside e2e dirs), `__tests__/`, `spec/**/*_spec.rb`, `tests/**/test_*.py`,
  `*_test.go`, `#[cfg(test)]` modules, `src-tauri/tests/`. Follow one level of callers: a test of the
  caller that drives the changed branch counts as `indirect`.
- **E2E**: the repo's own e2e suites (`playwright.config.*` → its `testDir`, `e2e/`, `cypress/`) plus the
  known e2e map below. Match by route, page/component name, `data-testid`, API path or user-visible text
  that the unit renders or serves.

### Known e2e map

E2E suites that live outside the PR's repo. Read them from the existing clone; never clone. If the clone
is missing, note `e2e: <repo> clone missing` and score e2e from the same repo only.

| PR repo | Also search | What it covers |
|---|---|---|
| `cardiologs-back` / `back` | `~/projects/front/playwright/test/` | API endpoints and jobs driven through the Holter UI |
| `cardiologs-front` / `front` | `~/projects/front/playwright/test/` (its own) | Holter app, login portal, min2 |
| `e2e-lib` | its consumers: `~/projects/front/playwright/test/`, `~/projects/platane/playwright/tests/`, `~/projects/resting/playwright/tests/` | shared Playwright fixtures and helpers |

Unknown repo: same repo only. Edit this table when a repo gets its own e2e suite elsewhere.

### Classify

For each unit, pick the strongest that holds:

- `direct`: a test calls the unit and asserts on the behaviour the hunk changed
- `indirect`: reached through a caller's test that exercises the changed branch
- `e2e-only`: only an e2e scenario walks through it
- `none`: nothing found

Name the tests (path + test name) you counted. "A test imports the file" is not `direct`: the test must
reach the changed lines. When in doubt, pick the weaker class.

Also mark `user_flow: true` for units on a user-visible flow (UI, public API, report output, a job a
user waits on). Only those count towards the e2e score.

## 3. Refactor red flags

Look for these in every PR, and weigh them heavily when `change_kind = refactor`:

- **Assertions changed** next to the code they guard (a refactor shouldn't need new expected values)
- **Tests deleted, skipped or `.only`'d** (`it.skip`, `xit`, `@pytest.mark.skip`, `#[ignore]`, `pending`)
- **A new mock on the seam being refactored**: the test stubs the very thing that moved, so it no longer
  proves behaviour survived
- **Bulk snapshot updates** with no explanation in the PR
- **A new code path** (branch, error case, flag) with no test

Each red flag is a `refactor-risk` (or `test-changed` for the first two) finding.

## 4. Optional measured run (unit tests only)

Only when **all** hold: dependencies are already installed in `WT` (`node_modules/`, `.venv/`,
`vendor/bundle`, or a `uv`-managed project where `uv run` syncs itself), a command below fits the repo, and
it finishes in under 10 minutes. **Never install anything, never run e2e, never run the full suite of a
big monorepo.** Limit the run to tests related to the changed files.

| Repo has | Command |
|---|---|
| `ubt.toml` with a coverage verb | `ubt test --coverage` (or the verb it defines) |
| jest | `npx jest --coverage --findRelatedTests <changed files>` |
| vitest | `npx vitest run --coverage <related test files>` |
| pytest (uv) | `uv run pytest --cov=<package> <related test files>` |
| cargo + `cargo llvm-cov` installed | `cargo llvm-cov --json -- <test filter>` |
| rspec | `bundle exec rspec <related spec files>` (SimpleCov if configured) |

Read the coverage report and compute the share of **changed lines** (RIGHT side of the hunks) that ran.
Record `measured: {"changed_lines_pct": N, "command": "…", "files": [...]}`, or
`measured: {"skipped": "<reason>"}` (deps missing, no fitting command, timeout, failing tests). Tests that
fail on the head are themselves a finding (`weak-test`, worst severity).

## 5. Score

Write the arithmetic out in `units` so the score can be checked.

- **Risk weight** per unit, from the hunk's tier: `critical` 3, `review` 2, `skim` 1 (`trivial` isn't a unit).
- **Unit score** (0–100) = Σ weight × credit / Σ weight, with credit `direct` 1.0, `indirect` 0.6,
  `e2e-only` 0.3, `none` 0. A unit with a `weak-test` finding (§6) gets at most 0.6, even if a test
  calls it: the score and the findings must agree. When a measured run exists, the files it covered use
  `changed_lines_pct / 100` as their credit instead of the static class.
- **E2E score** (0–100) = Σ weight × [an e2e scenario walks the changed behaviour] / Σ weight, over
  `user_flow` units only. A scenario that only visits the page doesn't count.
  `null` when no unit is on a user flow.
- **Overall** = 0.6 × unit + 0.4 × e2e (unit alone when e2e is `null`), minus 10 per refactor red flag
  (at most −30), floored at 0, rounded.
- **Band**: ≥ 80 `well covered`, 50–79 `partial`, < 50 `weak`.
- **Sanity check before writing**: a 100 means every unit is `direct` with no `weak-test`, `untested`
  or red-flag finding on it. If your findings say otherwise, recompute.

## 6. Focus areas and findings

**Focus areas** (≤ 5, riskiest first): one line each, a place the reviewer should read hardest
*because* tests won't catch a mistake there, e.g. "`retryUpload` backoff: new branch, no test, critical".

**Findings** (≤ 12, riskiest first). Common shape from `critique.md` §4, with:

- `kind`: `untested` (unit `none`/`e2e-only` on a `review`/`critical` hunk), `weak-test` (a test exists
  but doesn't assert the changed behaviour, or fails), `refactor-risk` (a §3 red flag), `test-changed`
  (assertions changed or tests removed alongside the code)
- `file`/`line`/`side`: the changed line of the unit (RIGHT), or the test line for `test-changed`
- `ref`: the tests you found (or "no test found in <dirs searched>")
- `fix`: **the concrete test to write**: where (file), its name, and what it asserts. For a refactor,
  prefer a characterisation test pinned on the *base* behaviour.
- `tier`: the hunk's tier you scored with (the merge agent uses it to decide comment vs report-only)

Gaps on `skim` hunks still count in the score. List them as findings only if there's room left.

## 7. Output

Write `/tmp/cr-RID/coverage.json` with the Write tool:

```json
{
  "skipped": null,
  "change_kind": "refactor",
  "score": { "overall": 62, "unit": 70, "e2e": 50, "band": "partial", "red_flags": 1,
             "measured": { "skipped": "node_modules missing in worktree" } },
  "summary_line": "Coverage: 62/100 (unit 70, e2e 50, partial). Focus: retryUpload backoff, upload error toast",
  "focus_areas": ["retryUpload backoff: new branch, no test (critical)"],
  "units": [
    { "file": "src/uploader.ts", "symbol": "retryUpload", "hunk_id": "3f9a1c0b2d4e", "tier": "critical",
      "weight": 3, "coverage": "none", "credit": 0, "user_flow": true, "e2e": false, "tests": [] }
  ],
  "findings": [
    { "file": "src/uploader.ts", "line": 42, "side": "RIGHT", "tier": "critical",
      "title": "Backoff branch is untested",
      "body": "The new exponential backoff only runs after a 5xx; no unit test reaches it and the e2e suite never fails an upload.",
      "kind": "untested", "ref": "no test found in src/**/*.test.ts, playwright/test/",
      "fix": "src/uploader.test.ts › 'retries with backoff on 503': mock fetch to fail twice, assert 3 calls and the delays." }
  ]
}
```

`summary_line` must start with `Coverage:` and fit one line: the merge agent stores it in the focus
summary, replacing the previous run's line.

## Report (at most 150 words)

The overall score with its sub-scores and band, `change_kind`, the measured run (or why skipped), the
top focus area, and the finding count per kind. No full findings.
