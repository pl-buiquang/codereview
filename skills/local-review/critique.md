# Critique layer (`--critique`)

Two independent axes, each run by its own sub-agent so neither pollutes the other's context:

- **Standards**: does the change follow this repo's documented coding standards (plus a smell baseline)?
- **Spec**: does the change do what the originating issue/spec asked for?

A change can pass one axis and fail the other (right behaviour, wrong conventions; or clean code that
implements the wrong thing). Report them separately and **never merge or rerank findings across axes**.

## 1. Gather inputs once (main agent)

- **Diff**: `cr diff RID` (already validated non-empty in Step 0). The sub-agents get the review id and
  may run `cr diff RID` / `cr show RID --json` themselves; paste the diff only if it is small.
- **Checkout**: if `--codex` created one, or the cwd is a clone of the repo, use it for reading files. Otherwise
  run `cr worktree RID` once and use that path. Sub-agents read repo files from there.
- **Spec source**, first hit wins:
  1. GitHub PR: `gh pr view <n> -R <owner>/<repo> --json title,body,closingIssuesReferences`, then
     `gh issue view <num> -R <owner>/<repo> --json title,body` for each closing issue. Cross-repo issues
     use their own repo.
  2. Issue references (`#123`, `Closes #45`) in the commit messages (`git -C <checkout> log <base>..<head> --format=%B`).
  3. A path the user gave, or a file under `specs/`, `docs/` or `.scratch/` matching the branch name.
  4. None: skip the Spec agent and record "no spec available" for the report. Do not guess a spec from the diff.
- **Standards sources**: every file that documents how code is written, read from the checkout:
  `CLAUDE.md`, `AGENTS.md`, `CONTRIBUTING.md`, `CODING_STANDARDS.md`, style guides under `docs/`, and
  lint/format configs (only to know what tooling already enforces). If the checkout has none, the
  smell baseline below is the only standard.

## 2. Launch both agents in one message (foreground, parallel)

Use two `Agent` calls in a single message. Give each the review id, the checkout path, its sources and its
brief. Neither agent edits anything or calls `cr comment`; they only return findings.

### Standards agent brief

> Review the diff of review RID against the standards below. Report (a) every place the diff breaks a
> **documented** standard: cite the file and the rule; these may be hard violations. (b) every **smell** from
> the baseline you can defend: name it as "possible <smell>" and point at the hunk. Smells are always
> judgement calls. A documented repo standard overrides the baseline: where the repo endorses a pattern,
> don't flag it. Skip anything linters, formatters or the compiler already enforce. Return only the JSON array
> below, at most ~12 findings, most important first.

Smell baseline to paste into the brief (Fowler, *Refactoring* ch. 3). Each reads *what it is* → *how to fix*:

- **Mysterious Name**: a function, variable or type whose name doesn't reveal what it does or holds. → rename it; if no honest name comes, the design is murky.
- **Duplicated Code**: the same logic shape appears in more than one hunk or file of the change. → extract the shared shape.
- **Feature Envy**: a method that reaches into another object's data more than its own. → move it onto the data it envies.
- **Data Clumps**: the same few fields or params keep travelling together. → bundle them into one type.
- **Primitive Obsession**: a primitive or string standing in for a domain concept. → give the concept its own small type.
- **Repeated Switches**: the same `switch`/`if` cascade on the same type recurs across the change. → polymorphism, or one shared map.
- **Shotgun Surgery**: one logical change forces scattered edits across many files. → gather what changes together into one module.
- **Divergent Change**: one file or module is edited for several unrelated reasons. → split so each module changes for one reason.
- **Speculative Generality**: abstraction, parameters or hooks added for needs the spec doesn't have. → delete it.
- **Message Chains**: long `a.b().c().d()` navigation the caller shouldn't depend on. → hide the walk behind one method.
- **Middle Man**: a class or function that mostly delegates onward. → call the real target directly.
- **Refused Bequest**: a subclass or implementer that ignores or overrides most of what it inherits. → composition instead of inheritance.

### Spec agent brief

> Review the diff of review RID against the spec below. Report (a) requirements the spec asked for that are
> **missing or only partly done**; (b) behaviour in the diff that **wasn't asked for** (scope creep); (c)
> requirements that look implemented but whose implementation looks **wrong**. Quote the spec line for each
> finding. Don't judge style. Return only the JSON array below, at most ~12 findings, most important first.

## 3. Findings shape (both agents)

```json
[
  {
    "file": "src/uploader.ts",
    "line": 42,
    "side": "RIGHT",
    "title": "Retry loop is unbounded",
    "body": "One or two sentences: what is wrong and why it matters.",
    "kind": "violation | smell | missing | scope-creep | wrong",
    "ref": "CLAUDE.md: 'Errors are returned, never thrown'  /  spec: 'retry at most 3 times'",
    "fix": "One concrete change."
  }
]
```

`line` and `file` are `null` for findings that don't belong to one place (a missing requirement that has no
code yet). Those go into the review body only; see SKILL.md "Merge findings".

## 4. After the agents return

- Keep each agent's findings in its own bucket (`standards`, `spec`). Validate each finding against the diff:
  drop any whose `file`/`line` isn't in the diff (unless `null`).
- If the Spec agent was skipped, say so in the report and the review body.
