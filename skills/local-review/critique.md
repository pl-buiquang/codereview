# Critique layer (`--critique`)

Two independent axes, each run by its own fresh-context agent so neither pollutes the other's context:

- **Standards**: does the change follow this repo's documented coding standards (plus a smell baseline)?
- **Spec**: does the change do what the originating issue/spec asked for?

A change can pass one axis and fail the other (right behaviour, wrong conventions; or clean code that
implements the wrong thing). Report them separately and **never merge or rerank findings across axes**.

## 1. Launch (main context)

SKILL.md Step 3 launches both agents in one message with `Agent` calls (foreground, parallel), together
with the other layers. Each brief carries the common header (review id, checkout `WT`, scratch dir, skill
dir) plus "read `critique.md`; you are the Standards (Spec) agent". The main context gathers nothing and
pastes no diff.

Neither agent edits anything or calls `cr comment`; they only write their findings file and report.

## 2. Gather inputs (each agent, for itself)

- **Diff**: `cr diff RID`; `cr show RID --json` for metadata.
- **Checkout**: read repo files from `WT`. If `WT` is empty, say so in the findings (`skipped` for
  Standards file reads; line-anchored findings still work from the diff).
- **Spec source (Spec agent)**, first hit wins:
  1. GitHub PR: `gh pr view <n> -R <owner>/<repo> --json title,body,closingIssuesReferences`, then
     `gh issue view <num> -R <owner>/<repo> --json title,body` for each closing issue. Cross-repo issues
     use their own repo.
  2. Issue references (`#123`, `Closes #45`) in the commit messages (`git -C <checkout> log <base>..<head> --format=%B`).
  3. A path the caller gave, or a file under `specs/`, `docs/` or `.scratch/` matching the branch name.
  4. None: write `{"skipped": "no spec available", "findings": []}` and stop. Do not guess a spec from the diff.
- **Standards sources (Standards agent)**: every file that documents how code is written, read from the
  checkout: `CLAUDE.md`, `AGENTS.md`, `CONTRIBUTING.md`, `CODING_STANDARDS.md`, style guides under `docs/`,
  and lint/format configs (only to know what tooling already enforces). If the checkout has none, the smell
  baseline below is the only standard.

## 3. Briefs

### Standards agent

> Review the diff of review RID against the standards below. Report (a) every place the diff breaks a
> **documented** standard: cite the file and the rule; these may be hard violations. (b) every **smell** from
> the baseline you can defend: name it as "possible <smell>" and point at the hunk. Smells are always
> judgement calls. A documented repo standard overrides the baseline: where the repo endorses a pattern,
> don't flag it. Skip anything linters, formatters or the compiler already enforce. At most ~12 findings,
> most important first.

Smell baseline (Fowler, *Refactoring* ch. 3). Each reads *what it is* → *how to fix*:

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

### Spec agent

> Review the diff of review RID against the spec below. Report (a) requirements the spec asked for that are
> **missing or only partly done**; (b) behaviour in the diff that **wasn't asked for** (scope creep); (c)
> requirements that look implemented but whose implementation looks **wrong**. Quote the spec line for each
> finding. Don't judge style. At most ~12 findings, most important first.

## 4. Output

Each agent writes its file (`/tmp/cr-RID/standards.json` or `/tmp/cr-RID/spec.json`) and reports only the
finding count and its worst finding (at most 150 words, no full findings). The file is:

```json
{
  "skipped": null,
  "findings": [
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
}
```

`skipped` is a reason string when the agent could not run (see Spec source 4), else `null`. `line` and
`file` are `null` for findings that don't belong to one place (a missing requirement that has no code yet).
Those go into the merge report only. The merge agent (`merge.md`) validates findings against the diff,
anchors them, tags them `ai:review` and posts them.
