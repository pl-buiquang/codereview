# The `cr` CLI and deep links

`cr` is a standalone binary ([`src-tauri/src/bin/cr.rs`](../../src-tauri/src/bin/cr.rs)) that reads and writes the **same SQLite database as the app**. Edits made with `cr` show up in the app on its next read, and vice versa. It is the interface for scripts and for AI agents (the [`/local-review` skill](focus-and-ai.md#the-local-review-skill) and the in-app chat both drive it).

`cr --help` and `cr <command> --help` are the reference; this page is a map.

## Global options

| Option | Meaning |
|---|---|
| `--db <path>` | SQLite file to use. Overrides `CODEREVIEW_DB` and the default location ([where the data lives](settings.md#where-the-data-lives)) |
| `--json` | Machine-readable output instead of text. Put it after the subcommand (`cr show 12 --json`) |
| `--version` | Version of the build |

Environment: `CODEREVIEW_DB` (database path), `CODEREVIEW_BASE_PATHS` (comma-separated directories searched for a local clone by `cr create pr`; the app uses the Settings value instead). Errors go to stderr with exit code 1.

`gh` (authenticated) is needed for anything touching GitHub; `git` for local reviews.

## Installing `cr`

The app bundles `cr` as a sidecar and installs it on launch ([`cr_install.rs`](../../src-tauri/src/cr_install.rs)). The destination is the first of:

1. wherever `cr` already is on `PATH` (replaced in place);
2. `~/.cargo/bin` (or `$CARGO_HOME/bin`) if it exists;
3. `~/.local/bin/cr` (macOS/Linux; may need adding to `PATH`) or `%LOCALAPPDATA%\CodeReview\bin\cr.exe` (Windows).

The copy is skipped when the installed file has the same size as the bundled one, and does nothing in dev builds (no sidecar). When something was installed or updated, the app logs `cr CLI installed|updated -> <path>`.

From a source checkout instead:

```bash
cargo install --path src-tauri --bin cr
```

## Reviews

| Command | Description |
|---|---|
| `cr list [--repo <id>]` | List reviews (id, repo, target, verdict, comment count, status) |
| `cr show <id>` | Review detail: summary, comments with replies, draft thread replies |
| `cr diff <id>` | Print the review's diff |
| `cr worktree <id>` | Print the path of a checkout of the review's head branch, creating a worktree if none holds it. Needs a local clone |
| `cr create local <repo-id> <base> <head> [--three-dot] [--reuse-draft]` | Review of a local diff between two refs (a virtual PR) |
| `cr create pr <url \| owner name number> [--reuse-draft]` | Review of a GitHub PR |
| `cr update <id> [--body <text>] [--verdict approve\|comment\|request_changes]` | Set the summary and/or verdict |
| `cr delete <id>` | Delete a draft review |
| `cr export <id> [--format markdown\|md\|json] [-o <file>]` | Export a review (stdout by default) |

```bash
cr list --repo 3 --json
cr create pr https://github.com/acme/widget/pull/99 --reuse-draft --json
cr create local 3 main my-branch --three-dot
cr update 12 --body "Looks good overall" --verdict comment
cr export 12 --format json -o review.json
```

## Comments

`cr comment <action>`. Tagged comments are never published; `--tag` takes `ai`, `ai:explain`, `ai:review`, `ai:codex`, `local` or `none`, and **defaults to `local`**, so CLI comments stay off GitHub until you untag them (`cr comment tag <id> --tag none`) or edit them in the app.

| Command | Description |
|---|---|
| `add <review-id> --file <p> --line <n> --body <t> [--side LEFT\|RIGHT] [--tag ..]` | Line comment (`RIGHT` = new line, default) |
| `add-file <review-id> --file <p> --body <t> [--tag ..]` | Whole-file comment |
| `update <comment-id> --body <t>` | Change a comment's body |
| `tag <comment-id> --tag <tag>` | Set or clear the tag |
| `delete <comment-id>` | Delete a comment |
| `resolve <comment-id>` / `unresolve <comment-id>` | Resolve or reopen a thread |
| `reply <review-id> --parent-id <comment-id> --body <t> [--tag ..]` | Reply inside a local comment thread |
| `draft-reply <review-id> --thread-comment-id <n> --file <p> --line <n> --body <t> [--side ..]` | Draft a reply to an existing **GitHub** thread; nothing is posted yet |
| `publish-replies <review-id>` | Post all draft replies for the review to GitHub |
| `publish-reply <comment-id>` | Post one draft reply |

```bash
cr comment add 12 --file src/api.ts --line 40 --body "Why not reuse fetchUser?"
cr comment add 12 --file src/api.ts --line 40 --body "Keep" --tag none      # publishable
cr threads acme widget 99 --json                  # find thread comment ids
cr comment draft-reply 12 --thread-comment-id 123456 --file src/api.ts --line 40 --body "Fixed in abc123"
cr comment publish-replies 12
```

`cr threads <owner> <name> <number>` lists a PR's GitHub review threads (thread id, location, resolved state, comments with their `databaseId`).

## Repositories

| Command | Description |
|---|---|
| `cr repo list` | Registered repositories (id, remote, local path or "(remote-only)") |
| `cr repo add <path>` | Register a git repository |
| `cr repo remove <id>` | Unregister |

## Sync, Inbox and Board

| Command | Description |
|---|---|
| `cr sync [--full \| --light]` | Sync the GitHub mirror (Inbox + Board): discovery, then refetch what changed. `--full` refetches everything and fetches every local clone; `--light` only re-checks known items |
| `cr inbox list` | The Inbox's New queue |
| `cr inbox archive <ref>` | Archive an item (`<ref>` = GitHub URL or `owner/repo#n`) |
| `cr inbox restore <ref>` | Bring an archived item back to New |
| `cr inbox sync` (alias `refresh`) | Sync the inbox only |
| `cr board list [--lane backlog\|in_progress\|submitted\|reviews\|done]` | Cards by lane |
| `cr board show <ref> \| --here` | One card. `--here` resolves it from the current repo and branch (the branch's open PR, else an `issue-<n>` branch's issue) |
| `cr board track <ref>` | Track an issue or PR |
| `cr board link <issue-card-id> <pr-ref>` | Link a PR to a backlog issue card (ids from `cr board list --json`) |
| `cr board sync` (alias `refresh`) | Sync the board only |
| `cr board meta add <ref> \| --here --kind file\|link\|note\|claude_session --value <v> [--tag <t>] [--label <l>] [--cwd <dir>] [--track]` | Attach metadata. `--track` tracks the card first if needed; `--value` may be omitted for a note that has a `--tag` |
| `cr board meta list <ref> \| --here` | A card's metadata |
| `cr board meta rm <id>` | Delete a metadata item |

```bash
cr sync --light
cr inbox list --json
cr inbox archive acme/widget#7
cr board list --lane reviews
cr board show --here
cr board meta add --here --kind file --value ~/plans/widget.md --tag plan
cr board meta add --here --kind claude_session --value "$SESSION_ID" --cwd "$PWD"
```

See [`skills/board/SKILL.md`](../../skills/board/SKILL.md) for the agent workflow.

## Focus

`cr focus hunks|hunk|set|show|clear <review-id>`: the scripting side of [Focus review](focus-and-ai.md#scripting-with-cr-focus).

| Command | Description |
|---|---|
| `cr focus hunks <id>` | The review's hunks: id, file, header, +/- counts, rule tier, preview |
| `cr focus hunk <id> <hunk-id>` | One hunk's full text |
| `cr focus set <id> --file <plan.json\|-> [--level off\|thorough\|focus\|critical]` | Write a focus plan, replacing the previous one (manual overrides are kept) |
| `cr focus show <id>` | Status, level, model, stale marker, summary, tier counts, AI notes |
| `cr focus clear <id>` | Delete the focus |

## Opening the app

`cr open ...` builds a `codereview://` URL and hands it to the OS, which launches or focuses the app.

| Command | Opens |
|---|---|
| `cr open review <id>` | An existing review |
| `cr open pr <owner> <name> <number>` | A review for a GitHub PR (created if needed) |
| `cr open diff <repo-id> <base> <head> [--three-dot]` | A review for a local diff (created if needed) |
| `cr open graph [path] [--ref <name> \| --no-ref] [--repo <id>]` (alias `tree`) | A repo's commit Graph |
| `cr open home [section]` | The Home tab, optionally a section |
| `cr open url <url>` | Any raw `codereview://` URL |

`cr open diff` always writes `three_dot=` into the URL: without `--three-dot` it is `false` (two-dot), even though the flag's help text says the default is true. Pass `--three-dot` for merge-base semantics.

`cr open graph` works on the repo containing `path` (default: the current directory), registering it if it isn't yet. By default it focuses the checked-out branch (or the exact tag when detached) and, inside a linked worktree, selects that worktree as the graph's context.

```bash
cr open graph                    # current repo, current branch
cr open graph --ref v1.2.0       # a branch, remote branch (origin/x) or tag
cr open graph --no-ref           # just the repo's Graph
cr open graph ~/projects/foo
cr open home inbox
cr open url "codereview://board/acme/widget/7"
```

## Deep links

The app registers the `codereview://` scheme ([`deep_link.rs`](../../src-tauri/src/deep_link.rs) parses, [`deepLink.ts`](../../src/lib/deepLink.ts) acts). If the app is already running, the URL is forwarded to that instance. A malformed URL shows an "Invalid deep link" toast.

| URL | Action |
|---|---|
| `codereview://review/<review_id>` | Open the review (refreshes its cached data first) |
| `codereview://pr/<owner>/<name>/<number>` | Create or open a review for the GitHub PR (uses the Settings repo base paths to find a clone) |
| `codereview://diff?repo=<id>&base=<ref>&head=<ref>[&three_dot=false]` | Create a review for a local diff. `three_dot` defaults to true; `false` or `0` selects two-dot. Needs a repo with a local clone. Refs are validated (no leading `-`, no control characters) |
| `codereview://repo/<repo_id>[?ref=<name>][&worktree=<path>]` | Open the repo's Graph, optionally focused on a branch or tag and using a worktree as context. Empty values are ignored |
| `codereview://board/<owner>/<name>/<number>` | Show that card on the Board (toast if it isn't tracked) |
| `codereview://home` | Home tab |
| `codereview://home/<section>` | Home section: `sessions`, `board`, `inbox`, `reviews`, `worktrees`, `repositories`, or `archive` (the Inbox's Archive tab). An unknown section just opens Home |

Anything else, or a non-numeric id, is rejected.
