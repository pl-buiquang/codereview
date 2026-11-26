# Readiness agent (own PRs)

Runs with the critique layer when the PR or branch is the caller's own. It checks what the author's agent
tends to get wrong before a reviewer sees the PR. It does not judge code quality (Standards), requirements
(Spec), bugs (Codex) or tests (Coverage): leave those to the other layers.

## Inputs

- **Diff and metadata**: `cr diff RID`, `cr show RID --json`.
- **PR body**: `gh pr view <n> -R <owner>/<repo> --json title,body,baseRefName,headRefName,files`. Branch
  mode has no body: skip the body check and say so.
- **Stack**: when the base is not `main`/`master`, `gh pr list -R <owner>/<repo> --author @me --state open --json number,title,baseRefName,headRefName`
  gives the parent (the PR whose head is this base) and the children (PRs based on this head).
- **Per-repo notes file**: the one the user's CLAUDE.md points to, if any. It may name repo-specific
  scope or vocabulary rules.
- **Checkout**: `WT`, for grepping the repo. Empty `WT`: say which checks you skipped.

## Checks

1. **Body vs diff** (`kind: body-drift`)
   - Every claim in the body matches the diff: no described change that isn't there, no file category in
     the diff (migrations, generated files, CI, config, contract or API changes) that the body doesn't
     mention.
   - Scope that was dropped is gone from the body.
   - The body leads with the user impact and the reason, and says how to check the change (QA path).
   - No local paths (`~/scratch`, `/tmp`, `/Users/`) or placeholder text.
2. **Out-of-scope files** (`kind: out-of-scope`)
   - `.github/` changes in a feature PR, lockfile changes without a manifest change, snapshot or generated
     files the change doesn't explain, build artefacts, formatting-only hunks in files the change is not
     about.
3. **Stack shape** (`kind: stack`), only for stacked PRs
   - A new exported symbol, route, endpoint or config key with no caller in this PR.
   - Edits to a file that a child PR deletes or moves.
   - A PR whose title or body says move/rename/relocate but whose moved files have content changes
     (`git -C WT diff -M --stat <base>...HEAD` shows changes, not only renames and import edits).
   - Tests for code in this PR that land in another PR of the stack.
4. **Vocabulary** (`kind: vocabulary`)
   - New domain nouns in identifiers, user-facing copy or docs that appear nowhere else in the repo
     (`git -C WT grep -i`) and not in the linked issue or `GLOSSARY.md`.
   - UI wording in backend code, unexplained abbreviations, comments describing code that was removed.
   - READMEs or docs that restate general knowledge instead of repo-specific steps.
5. **Sibling spec** (`kind: sibling-spec`)
   - For each new test file, compare it with one existing test file of the same kind in the same lib or
     folder: structure, setup, naming and helpers. Flag a clear divergence only.

## Output

Write `/tmp/cr-RID/readiness.json` in the critique format (`critique.md` §4), with `kind` one of
`body-drift | out-of-scope | stack | vocabulary | sibling-spec`. Findings about the body have
`file: null` and `line: null`; put the quoted body line in `ref`. At most ~12 findings, most important
first. Report only the finding count and the worst finding (at most 150 words).
