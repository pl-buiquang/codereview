# Focus and AI features

Everything AI in CodeReview runs through a harness, the [`claude` CLI](https://docs.claude.com/en/docs/claude-code) (`claude -p`), [opencode](https://opencode.ai) (`opencode run`) or [Codex](https://github.com/openai/codex) (`codex exec`); opencode also reaches OpenRouter and other API-key providers. Nothing is bundled and the app stores no API key.

- **Auth** is whatever your `claude` install already uses (Claude subscription, API key, Bedrock, ...). The app does not pass `--bare` or an empty `--setting-sources`, so Claude reads its own auth and settings from `~/.claude/`. It does pass `--strict-mcp-config` (no MCP servers) and an explicit `--permission-mode acceptEdits`, so your own default permission mode never leaks in.
- If `claude` is not on `PATH`, the AI actions are disabled (Settings → General → Environment shows what was found).
- AI-written content is **local only**: focus tiers, the focus summary and comments tagged `ai`, `ai:explain`, `ai:review`, `ai:codex` or `ai:coverage` are never published to GitHub (see [AI notes and tags](#ai-notes-and-tags)).

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

The AI can attach a short note to a hunk. Notes are ordinary **draft comments tagged `ai:explain`**. Each comment's tag menu offers `None`, `AI`, `Explain`, `Review`, `Codex`, `Coverage` and `Local`. Tagged comments show a pill and are skipped when publishing. To keep a note as your own comment, set its tag to **None**; it is then published with the review like any other comment.

## The `/local-review` skill

The Claude Code skill in [`skills/local-review/`](../../skills/local-review/SKILL.md) creates the review through the `cr` CLI, fans analysis out to fresh-context agents, and opens the app on the result. Install it once; the symlink keeps it in sync with the repo:

```bash
ln -s "$PWD/skills/local-review" ~/.claude/skills/local-review
```

It also needs `cr` on `PATH` (see [cli.md](cli.md#installing-cr)). Usage:

```text
/local-review <owner/repo#N | pr-url | branch <head> [--base <b>] [--repo <path>] | review-id> [--explain] [--depth normal|advanced | layers]
/local-review reply owner/repo#N
```

The skill does two separate jobs.

**Explain** (`--explain`, alias `--plain`) is a reading aid, not a review. It posts one `ai:explain` comment
per change ([`explain.md`](../../skills/local-review/explain.md)) at any PR size, never touches the focus,
and a rerun replaces the explanations you haven't touched.

**Review** runs at two depths:

| Depth | Layers |
|---|---|
| `--depth normal` | Standards and Spec sub-agents ([`critique.md`](../../skills/local-review/critique.md), with a code-smell baseline) + test coverage in **static** mode (tests mapped, not run) |
| `--depth advanced` | normal + the Codex plugin's adversarial review of the checkout ([`codex.md`](../../skills/local-review/codex.md)) + coverage in **measured** mode (runs related unit tests when deps are installed) |

- **No flag** means a review: `advanced` for a large PR (> 600 lines or > 20 files) of yours, else `normal`.
- **Single layers** remain for terminal use: `--focus [--level ..]` (tiers + notes, like **Generate focus**),
  `--critique`, `--codex[=adversarial|native] ["focus text"]`, `--coverage` (measured), `--with-replies`
  ([`reply.md`](../../skills/local-review/reply.md)).
- A layer that can't run (no Codex plugin, no checkout, no test runner) is skipped and named in the report;
  the rest continue.

### How review findings work ([`merge.md`](../../skills/local-review/merge.md))

- **One severity scale.** Every finding is `critical`, `high`, `medium` or `low`: Codex keeps its own; Spec
  `wrong` is high, `missing` medium, `scope-creep` low; a documented Standards rule is medium, a smell low;
  coverage `refactor-risk` is high, `test-changed` medium, gaps medium on visible hunks and low elsewhere.
- **Deduplicated across layers.** Findings on the same hunk about the same problem become one comment that
  lists every source, e.g. `**[high · codex, standards · unbounded loop]**`. The tag follows the first source
  (`ai:codex` > `ai:review` > `ai:coverage`).
- **A rerun replaces the previous findings** of the layers it runs (`cr comment prune`). Comments you edited,
  resolved, replied to or untagged are yours and stay.
- **Tied to the focus.** A review needs a focus: it reuses a fresh one, else generates a **tiers-only** focus
  (no notes) in the same run. Findings then raise the hunks they land on: critical/high → `critical`,
  medium → at least `review`, low → no raise (a low finding on a hidden hunk stays in the report only). The
  focus summary gets a `Findings: …` line (and `Coverage: …`). Regenerating the focus keeps them visible:
  the focus prompt floors each hunk's tier from the severity prefix of the AI findings already on it. A small
  PR (< 150 lines and ≤ 5 files) gets no focus, since nothing would be hidden.
- Comments are tagged `ai:explain` / `ai:review` / `ai:codex` / `ai:coverage`; the verdict is never set.
- The coverage score is the `Coverage: …` summary line; a rerun replaces it.
- The app embeds its own copy of `focus.md` at build time; it is the default of the editable "Focus mode"
  prompt. A prompt override does not get later changes to it (such as the findings floor).
- `reply` mode drafts local replies to GitHub threads. Publish them with "Publish N draft replies" in the app or `cr comment publish-replies`.

## In-app AI runs

The review actions menu launches the skill headlessly in the background, one run per review at a time:

| Menu item | Runs |
|---|---|
| Explain changes | `/local-review <id> --explain` |
| AI review | `/local-review <id> --depth normal` |
| AI review: advanced | `/local-review <id> --depth advanced` (needs a local clone). Its allow-list adds test runners (`ubt`, `uv run`, `npx jest`/`vitest`, `pnpm exec`, `cargo llvm-cov`, `bundle exec rspec`) for the measured coverage run, which never installs dependencies |

Behavior ([`ai_review.rs`](../../src-tauri/src/commands/ai_review.rs)):

- Needs `claude` and the skill installed at `~/.claude/skills/local-review`.
- Disabled on published reviews and while another AI run is active on the same review.
- Runs on the routine's profile (Sessions → Config). Runs in the repo checkout (a temp dir without a clone) with a restricted tool allow-list (`Read`, `Grep`, `Bash(cr:*)`, `Bash(git:*)`, `Bash(gh pr view:*)`, ...). `CODEREVIEW_DB` is set so its `cr` calls hit the same database as the app.
- The header shows a spinner while it runs; results appear as tagged comments and raised tiers.
- These routines' prompt and tools follow the shipped skill: they are refreshed at every start (profile, model, timeout and enabled are yours). The former `critique`/`codex` routines became `AI review`/`AI review: advanced` with their settings and history; the `test coverage` routine's runs moved to advanced.

## Chat panel

A per-review conversation with Claude, in the review view ([`ChatPanel.tsx`](../../src/components/ChatPanel.tsx), [`chat.rs`](../../src-tauri/src/commands/chat.rs)).

- Needs a **local clone**. The chat runs in the PR branch's checkout (an existing worktree or one the app creates) where Claude can read, edit and commit. It is told not to push unless asked.
- The first turn starts a `claude -p` session; later turns `--resume` it. If the chat moves to a different checkout, a new session starts.
- The system prompt carries the PR title, repo, changed files, existing comments and the `cr` commands for the review, so Claude can read, add, update or resolve comments itself.
- A profile selector sits in the panel (the profile the chat runs on; the default comes from Settings → AI). A chat pins the profile it started with; picking another one starts a new session and the earlier messages are replayed to it once (capped at about 200 KB, oldest dropped first).
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

**Harnesses** shows whether each harness binary was found and its version.

**Profiles**: a profile is a harness plus a model (and harness-specific settings, never a secret). The *default* profile applies wherever nothing more specific is chosen. The model field accepts any name; Claude Code suggests `sonnet`, `opus`, `fable` and `haiku`. An empty model means the harness' own default, which for Claude Code falls back to `CODEREVIEW_CHAT_MODEL`, then `ANTHROPIC_DEFAULT_OPUS_MODEL`, then Claude's own default. The model of the former *Chat & focus model* and *Board overview model* fields was imported into the profiles once.

**Setting up opencode.** Install opencode, then add a provider in a terminal with `opencode auth login` (OpenRouter, an opencode subscription, any provider it supports); the app never sees the key. Add a profile with the *opencode* harness: the *Model* field searches the ids `opencode models` lists, grouped by provider (`openrouter/anthropic/claude-haiku-4.5`), and still accepts any text. *Refresh models* asks opencode again after you add a provider; with none configured the field says to run `opencode auth login` (copy button). The profile's settings JSON takes `agent` (an opencode agent, e.g. `build` or `plan`) and `extra_args` (a list of extra `opencode run` flags). Without an agent, the access level picks one: edit runs `build`, full runs `build --auto` (permissions auto-approved). opencode cannot deny shell commands, so it does not offer read-only access: a routine set to read-only is refused on an opencode profile (choose Edit or Full knowingly, or use a Claude profile). An `agent` in the settings or `--agent` in `extra_args` overrides the access-derived agent, so a `build` agent lets a routine edit. Routine runs use `--standalone` (a private opencode server per run) so stopping or timing out a run also stops the work. opencode has no system prompt flag, so the app puts it in front of the message; it has no tool allowlist (tool permissions belong to the agent) and the app does not fork its sessions. The cost in the Usage tab is what opencode reports for the step, and is 0 for free models.

**Setting up Codex.** Install the Codex CLI and sign in once in a terminal with `codex login` (ChatGPT account or API key); the app never sees the credentials. Add a profile with the *Codex* harness: the *Model* field suggests the models bundled with your Codex (`codex debug models`) and accepts any text; empty uses Codex's own default. The *Sandbox* and *Allow full access* controls edit the profile's settings JSON, which also takes `extra_args` (extra `codex exec` flags; any flag or config key that changes the sandbox or approvals, such as `--sandbox`, `--full-auto`, `-a`, `-p` or `-c sandbox_mode=...`, is refused). The run's access level picks the sandbox, and a profile *Sandbox* can only lower it (read-only < workspace-write < danger-full-access), never raise it: read only is a real read-only sandbox (writes fail, but everything stays readable), edit is `workspace-write` (the working directory, `/tmp` and the routine's extra directories; network access is Codex's default for that mode and not verified to be off, and no `git commit` inside a linked worktree), full access runs with **no sandbox** and only when the profile sets `allow_full`. Codex is non-interactive here, so it never asks for approval. Codex chooses its own session id (the app records it for chat resume and routine runs; cards are not linked to Codex sessions), has no system prompt flag (the app puts it in front of the message), no tool allowlist, and the app does not fork its sessions. Codex reports tokens but no cost, so the Usage tab shows 0 cost for it. Running `codex resume <id>` TUIs and TUIs that have sent a message show up in Sessions.

**Features** maps review chat, session summaries, theme suggestions and interactive sessions to a profile (empty: the default profile). Summaries and theme suggestions keep their fast built-in model unless the profile mapped to them sets one. Routines (focus, AI review, board insights, babysit...) pick their profile in Sessions → Config; a routine runs on its profile, else the default, and a launch can still override the model.

## Usage tracking (Settings → Usage)

Every run the app spawns through a harness is recorded in the local `ai_run` table (with the harness that ran it), failed ones included ([`ai_usage.rs`](../../src-tauri/src/ai_usage.rs)).

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
cr focus raise 12 --file raises.json  # only raise tiers + set a `Label:` summary line; notes/overrides kept
cr focus show 12                   # status, level, model, stale flag, summary, tier counts, notes
cr focus clear 12
```

Plan format: `{"summary": "...", "hunks": [{"id": "...", "tier": "critical", "reason": "...", "note": "..."}]}`. `--level off|thorough|focus|critical` on `set` also sets the display level; omit it to keep the user's choice. `cr focus hunks` warns above 500 hunks. See [cli.md](cli.md#focus) and `cr focus --help`.
