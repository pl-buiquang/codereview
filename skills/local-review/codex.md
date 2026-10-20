# Codex layer (`--codex`)

Runs the OpenAI Codex plugin's reviewer on the PR's checkout; the merge agent (`merge.md`) imports its
findings as `ai:codex` comments. The plugin's slash commands (`/codex:review`, `/codex:adversarial-review`)
can't be invoked from a skill, so call its companion script directly. It reviews a **local checkout**
(`base...HEAD`), not a PR number.

§1–3 run in the main context (cheap, no analysis). Parsing the result is the merge agent's job.

- `--codex` or `--codex=adversarial`: `adversarial-review`. Challenges the approach, assumptions and failure
  modes. Returns structured findings. Takes optional focus text (the quoted text after the flag).
- `--codex=native`: `review`, Codex's built-in reviewer. Returns **plain text only** (no structured findings,
  no focus text). It goes into the review body; it never becomes comments.

## 1. Locate the plugin

```bash
ls -d ~/.claude/plugins/cache/openai-codex/codex/*/scripts/codex-companion.mjs 2>/dev/null | sort -V | tail -1
```

If nothing is found, tell the user to install/run `/codex:setup`, write "codex skipped: plugin not
installed" to `/tmp/cr-RID/codex-skipped.txt`, and continue without this layer. Don't fail the review.

## 2. Get the checkout and base

```bash
cr worktree RID --json          # {"path": "..."}
cr show RID --json              # base_sha / head_sha on the review
git -C <path> rev-parse HEAD    # must equal head_sha
```

If `cr worktree` fails (no local clone), or HEAD differs from the review's `head_sha` (a reused worktree
that is stale or on another commit), skip the layer: write the reason to `/tmp/cr-RID/codex-skipped.txt`.
Never review the wrong commit.

## 3. Run it in the background

Start it in the same message as the agents; it is the slowest layer. Use Bash with `run_in_background: true`
and send stdout to the scratch dir:

```bash
node <companion.mjs> adversarial-review --cwd <worktree> --base <base_sha> --json "<optional focus text>" > /tmp/cr-RID/codex.json 2> /tmp/cr-RID/codex.err
# native:
node <companion.mjs> review --cwd <worktree> --base <base_sha> --json > /tmp/cr-RID/codex.json 2> /tmp/cr-RID/codex.err
```

Don't poll. You are notified when it finishes. If it exits non-zero, write "codex failed: <first line of
`/tmp/cr-RID/codex.err`>" to `/tmp/cr-RID/codex-skipped.txt` and continue without it. Do not read
`codex.json`: the merge agent parses it (payload shape and finding conversion are in `merge.md` §1).
