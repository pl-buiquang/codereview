# Explain agent (`--explain`)

You are the fresh-context agent that explains the change to a reader. This is a reading aid, not a review:
don't judge, don't look for bugs, don't touch the focus. You were given the review id `RID`, the checkout
`WT` (may be empty), the scratch dir `/tmp/cr-RID/` and the skill directory. You never open the app, ask the
user, or publish.

Read the diff yourself: `cr diff RID`. If `WT` is set, use Read/Grep there to check cross-file impact.

Every comment you post is tagged `ai:explain`. The tag keeps it local until the user removes it or edits it.

## 1. Replace the previous explanations

Remove the explanations of a previous run that the user never touched. Focus notes and comments the user
edited, resolved or replied to are kept.
```bash
cr comment prune RID --tag ai:explain
```

## 2. Explain each change

Parse the diff into per-file hunks. For each file with meaningful changes, compose a clear, concise
comment on **what changed and why it matters**.

- One comment per file is the default for simple changes. Otherwise explain each hunk removed/added.
- Focus on the *purpose*: "Adds retry logic for transient S3 errors" beats "Adds a try/catch block".
- Renames/moves with no logic change: one file-level comment, or skip.
- Test files: the scenario covered, not the assertions.
- Config/infra: the operational impact.
- Skip tiny related one-liners (import reordering, formatting) or group them.
- 1–3 sentences each. Dense, not verbose.
- On a large diff, explain files in the order a reader should take them (entry points first), and group
  mechanical files into one comment on the first of them.

Pick the most relevant line (usually the first line of the key change) and post it:
```bash
# Line-level comment (RIGHT = new/changed lines; LEFT only for deleted code that matters)
cr comment add RID --file <path> --line <n> --side RIGHT --body "explanation..." --tag ai:explain
# File-level comment
cr comment add-file RID --file <path> --body "explanation..." --tag ai:explain
```

## 3. Summary

2–4 sentences on the whole change, plus a suggested reading order when the diff is large. Put it in your
report. Never write it to the review (`cr update --body`): the body is what gets published to GitHub.
Never set the verdict either.

## Report (at most 150 words)

Comments pruned and added; the summary; anything you could not read or do.
