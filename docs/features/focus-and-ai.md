# Focus and AI features

Everything AI in CodeReview shells out to the [`claude` CLI](https://docs.claude.com/en/docs/claude-code) (`claude -p`). Nothing is bundled and the app stores no API key.

- **Auth** is whatever your `claude` install already uses (Claude subscription, API key, Bedrock, ...). The app does not pass `--bare` or an empty `--setting-sources`, so Claude reads its own auth and settings from `~/.claude/`. It does pass `--strict-mcp-config` (no MCP servers) and an explicit `--permission-mode acceptEdits`, so your own default permission mode never leaks in.
- If `claude` is not on `PATH`, the AI actions are disabled (Settings → General → Environment shows what was found).
- AI-written content is **local only**: focus tiers, the focus summary and comments tagged `ai`, `ai:explain`, `ai:review` or `ai:codex` are never published to GitHub (see [AI notes and tags](#ai-notes-and-tags)).

## Focus review

**Focus** is a lens over a review. Every hunk gets a tier, and the lower tiers collapse into one-click bars so you read what matters first.

| Tier | Meaning (rubric in [`skills/local-review/focus.md`](../../skills/local-review/focus.md)) |
|---|---|
| `critical` | Security/auth, data loss or migrations, concurrency, public API changes, money, hot-path error handling, subtle logic |
| `review` | Normal behavior changes, new logic, tests of new behavior |
| `skim` | Glue code, small refactors, logging, mechanical test updates |
| `trivial` | Formatting, renames/moves, imports, version bumps, generated output, lockfiles |

Some hunks are tiered by **rules** before any AI runs (lockfiles, binary, generated, snapshots and renames are `trivial`). The AI plan leaves them alone unless it disagrees.

### Display level

Chosen in the review header, independent of how the focus was generated.

| Level | Shows |
|---|---|
| Off | The full diff |
| Thorough | Everything except `trivial` |
| Focus | `critical` + `review` |
| Critical only | Only `critical` |

Hidden hunks become bars you can expand for this session or **Show at every level** (saved as an override). Hunks and whole files can also be hidden by hand. Hunks that have comments stay visible ("has comments"). A hunk that changed after the focus was generated is marked **new since focus**; when the head or base moved, the digest shows a stale warning with a **Regenerate** button.

### Digest

The **Focus** digest above the diff lists what is hidden at the current level, grouped by reason, with the AI's summary of the PR. Click an entry to reveal or jump to it. **Copy into summary** inserts the digest as markdown (`### Focus digest ...`) into the review summary; it is disabled on published reviews.

### Generating a focus

- **In the app**: **Generate focus** (**Regenerate focus** once a plan exists) in the review header. A popover asks for the **model** (Default / Sonnet / Opus / Fable). The run happens in the background (`claude -p`, in the repo checkout when there is one) and the header shows "Generating focus...". A failure is shown on the digest with the error. Generation has no level option: switch the display level any time with the level selector. **Regenerate AI focus** in the review actions menu does the same with the Settings → AI model.
- **From the terminal**: the `/local-review` skill, below.
- **From scripts or other agents**: `cr focus ...`, below.

### AI notes and tags

The AI can attach a short note to a hunk. Notes are ordinary **draft comments tagged `ai:explain`**. Each comment's tag menu offers `None`, `AI`, `Explain`, `Review`, `Codex` and `Local`. Tagged comments show a pill and are skipped when publishing. To keep a note as your own comment, set its tag to **None**; it is then published with the review like any other comment.

## The `/local-review` skill

The Claude Code skill in [`skills/local-review/`](../../skills/local-review/SKILL.md) creates the review through the `cr` CLI, fans analysis out to fresh-context agents, and opens the app on the result. Install it once; the symlink keeps it in sync with the repo:

```bash
ln -s "$PWD/skills/local-review" ~/.claude/skills/local-review
```

It also needs `cr` on `PATH` (see [cli.md](cli.md#installing-cr)). Usage:

```text
/local-review <owner/repo#N | pr-url | branch <head> [--base <b>] [--repo <path>] | review-id> [layers]
/local-review reply owner/repo#N
```

| Flag | Layer |
|---|---|
| `--focus [--level focus\|thorough\|critical]` | Tier every hunk ([`focus.md`](../../skills/local-review/focus.md)) |
| `--critique` | Standards and Spec sub-agents ([`critique.md`](../../skills/local-review/critique.md)), with a code-smell baseline |
| `--codex[=adversarial\|native] ["focus text"]` | The Codex plugin's review of the PR checkout, via `cr worktree` ([`codex.md`](../../skills/local-review/codex.md)) |
| `--plain` | One explanatory comment per change ([`explain.md`](../../skills/local-review/explain.md)) |
| `--with-replies` | Also draft replies to existing GitHub review threads ([`reply.md`](../../skills/local-review/reply.md)) |

- **No layer flag means auto mode**, chosen by effective size: small (< 150 lines and <= 5 files) is plain; medium is `--focus`; large (> 600 lines or > 20 files) is `--focus`, plus `--codex` if the PR is yours and `--critique` if it closes an issue.
- Comments are tagged `ai:explain` / `ai:review` / `ai:codex`; the verdict is always `comment`.
- The app embeds its own copy of `focus.md` at build time; it is the default of the editable "Focus mode" prompt.
- `reply` mode drafts local replies to GitHub threads. Publish them with "Publish N draft replies" in the app or `cr comment publish-replies`.

## In-app AI review runs

The review actions menu can launch the same skill headlessly in the background, one run per review at a time:

| Menu item | Runs |
|---|---|
| AI review: explain changes | `/local-review <id>` |
| AI review: critique (Standards + Spec) | `/local-review <id> --critique` |
| AI review: Codex adversarial | `/local-review <id> --codex` (needs a local clone) |

Behavior ([`ai_review.rs`](../../src-tauri/src/commands/ai_review.rs)):

- Needs `claude` and the skill installed at `~/.claude/skills/local-review`.
- Disabled on published reviews and while another AI review runs on the same review.
- Uses the Settings → AI chat model. Runs in the repo checkout (a temp dir without a clone) with a restricted tool allow-list (`Read`, `Grep`, `Bash(cr:*)`, `Bash(git:*)`, `Bash(gh pr view:*)`, ...). `CODEREVIEW_DB` is set so its `cr` calls hit the same database as the app.
- The header shows a spinner while it runs; results appear as tagged comments. There is no in-app `--focus` layer here: use **Generate focus**.

## Chat panel

A per-review conversation with Claude, in the review view ([`ChatPanel.tsx`](../../src/components/ChatPanel.tsx), [`chat.rs`](../../src-tauri/src/commands/chat.rs)).

- Needs a **local clone**. The chat runs in the PR branch's checkout (an existing worktree or one the app creates) where Claude can read, edit and commit. It is told not to push unless asked.
- The first turn starts a `claude -p` session; later turns `--resume` it. If the chat moves to a different checkout, a new session starts.
- The system prompt carries the PR title, repo, changed files, existing comments and the `cr` commands for the review, so Claude can read, add, update or resolve comments itself.
- A model selector sits in the panel; the default comes from Settings → AI.
- **Send to Chat**: in the line comment composer, this button sends your text, prefixed with the file, line range, side and the diff snippet, to the Chat panel instead of creating a comment. The panel expands if collapsed.
- Assistant replies show their output token count. Clear the chat to start over (required to pick up an edited Review chat prompt).

## Prompts and models (Settings → AI)

**Prompts** edits the system prompts the app uses. Edits autosave (stored in the `prompt_override` table); **Reset to default** removes the override. Edited prompts show a **Modified** badge and travel with the Settings export (Settings → General → Backup).

| Prompt | Used by | Notes |
|---|---|---|
| Live session summary | Sessions view "Generate summary" | |
| Board AI insights | Board overview | |
| Focus mode | Generate focus | An app-mode suffix (review id, `cr focus set` instructions) is always appended. Applies in the app only, not to the `/local-review --focus` skill |
| Review chat | First chat turn | Placeholders: `{{title}}`, `{{repo}}`, `{{pr_line}}`, `{{diff_summary}}`, `{{head_ref}}`, `{{worktree_path}}`, `{{comments}}`, `{{review_id}}`. Applies to new chats |

**Models**:

- *Chat & focus model*: passed as `--model` to `claude -p` (also used by in-app AI review runs). Empty falls back to `CODEREVIEW_CHAT_MODEL`, then `ANTHROPIC_DEFAULT_OPUS_MODEL`, then Claude's own default.
- *Board overview model*: default `sonnet`; runs with `--effort low` and no tools.

## Usage tracking (Settings → Usage)

Every `claude -p` run the app spawns is recorded in the local `ai_run` table, failed ones included ([`ai_usage.rs`](../../src-tauri/src/ai_usage.rs)).

- Range: Today / 7 days (default) / 30 days / All time.
- Totals: cost, runs (and failures), input, output and cache tokens, run time.
- Breakdowns **by feature** (Chat, AI review, Focus, Board overview, Session summary) and **by model**, plus a filterable list of recent runs showing what each was about (`repo#123` or `base...head`).
- Cost is the API-equivalent price `claude` reports; on a subscription it is not what you are billed.
- Codex runs launched by `/local-review --codex` are billed separately and are **not** tracked.

## Scripting with `cr focus`

```bash
cr focus hunks 12 --json           # hunk ids, files, +/- counts, rule tier, preview
cr focus hunk 12 <hunk-id>         # one hunk's full text
cr focus set 12 --file plan.json   # write a plan ("-" = stdin); manual overrides are kept
cr focus show 12                   # status, level, model, stale flag, summary, tier counts, notes
cr focus clear 12
```

Plan format: `{"summary": "...", "hunks": [{"id": "...", "tier": "critical", "reason": "...", "note": "..."}]}`. `--level off|thorough|focus|critical` on `set` also sets the display level; omit it to keep the user's choice. `cr focus hunks` warns above 500 hunks. See [cli.md](cli.md#focus) and `cr focus --help`.
