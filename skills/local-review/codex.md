# Codex layer (`--codex`)

Runs the OpenAI Codex plugin's reviewer on the PR's checkout and imports its findings. The plugin's
slash commands (`/codex:review`, `/codex:adversarial-review`) can't be invoked from a skill, so call its
companion script directly. It reviews a **local checkout** (`base...HEAD`), not a PR number.

- `--codex` or `--codex=adversarial`: `adversarial-review`. Challenges the approach, assumptions and failure
  modes. Returns structured findings. Takes optional focus text (the quoted text after the flag).
- `--codex=native`: `review`, Codex's built-in reviewer. Returns **plain text only** (no structured findings,
  no focus text). It goes into the review body; it never becomes comments.

## 1. Locate the plugin

```bash
ls -d ~/.claude/plugins/cache/openai-codex/codex/*/scripts/codex-companion.mjs 2>/dev/null | sort -V | tail -1
```

If nothing is found, tell the user to install/run `/codex:setup`, record "codex skipped: plugin not
installed" for the report, and continue without this layer. Don't fail the review.

## 2. Get the checkout and base

```bash
cr worktree RID --json          # {"path": "..."}
cr show RID --json              # base_sha / head_sha on the review
git -C <path> rev-parse HEAD    # must equal head_sha
```

If `cr worktree` fails (no local clone), or HEAD differs from the review's `head_sha` (a reused worktree
that is stale or on another commit), skip the layer and record why. Never review the wrong commit.

## 3. Run it in the background

Start it as soon as the review exists; it is the slowest layer. Use Bash with `run_in_background: true`:

```bash
node <companion.mjs> adversarial-review --cwd <worktree> --base <base_sha> --json "<optional focus text>"
# native:
node <companion.mjs> review --cwd <worktree> --base <base_sha> --json
```

Don't poll. You are notified when it finishes; collect stdout then. If it exits non-zero, record
"codex failed: <first line of stderr>" and continue without it.

## 4. Parse the result

stdout is one JSON payload.

**Adversarial**: the review is `payload.result` (`verdict`: `approve` | `needs-attention`, `summary`,
`findings`, `next_steps`). If `payload.result` is null (see `payload.parseError`), record
"codex returned unparseable output" and put `payload.rawOutput` in the review body under `## Codex`
instead of making comments. Each finding has `severity` (critical|high|medium|low), `title`, `body`,
`file`, `line_start`, `line_end`, `confidence` (0–1) and `recommendation`.

Convert each finding to the common shape in critique.md section 3:

```
file = file, line = line_start, side = RIGHT, title = title, body = body,
kind = "codex-<severity>", ref = "confidence <n>", fix = recommendation
```

Drop findings with `confidence < 0.3`. Keep `line_end` to write `L<start>-<end>` in file-level comments.
Keep `verdict` and `summary` for the review body (`## Codex`). `next_steps` are optional extra bullets
there; they are not comments.

**Native**: the text is `payload.codex.stdout`. Put it in the review body under `## Codex (native)`,
trimmed to its findings. No comments.
