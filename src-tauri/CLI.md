# `cr` — CodeReview CLI

Command-line interface for the CodeReview desktop app. Shares the same SQLite
database, so anything created via `cr` is immediately visible in the GUI and
vice versa.

## Install

```bash
cargo install --path src-tauri --bin cr
```

Requires `git` and `gh` (GitHub CLI, authenticated) on `PATH`.

## AI-assisted review workflow

The primary use case: let an AI analyze a GitHub PR and populate a review with
one explanatory comment per meaningful change, then browse the result in the
desktop app.

```bash
# 1. Create a review for the PR (prints the review ID)
cr create pr cardiologs back 1234

# 2. Dump the diff so the AI can read it
cr diff <review-id>

# 3. Add a comment per change (repeat for each file/hunk)
cr comment add <review-id> \
  --file src/api/handler.rs --line 42 --side RIGHT \
  --body "Extracts the retry logic into a helper — reduces duplication with the batch path."

# 4. Add whole-file-level comments for broader observations
cr comment add-file <review-id> \
  --file src/api/handler.rs \
  --body "This file was restructured to separate transport from business logic."

# 5. Set the review summary
cr update <review-id> --body "Refactors the handler layer to separate concerns."

# 6. Open the desktop app — the review is ready to browse
```

The `/local-review` Claude Code skill automates steps 1–5.

## Command reference

### Global flags

| Flag | Description |
|---|---|
| `--db <path>` | Path to the SQLite database (overrides `CODEREVIEW_DB` env and platform default) |
| `--json` | Output as JSON instead of human-readable tables |

### Reviews

```bash
cr list [--repo <id>]              # List reviews, optionally filtered by repo
cr show <review-id>                # Show review detail + comments
cr diff <review-id>                # Print the unified diff
cr create local <repo-id> <base> <head> [--three-dot]  # Review a local diff
cr create pr <owner> <name> <number>                    # Review a GitHub PR
cr update <review-id> [--body <text>] [--verdict approve|comment|request_changes]
cr delete <review-id>              # Delete a draft review
cr publish <review-id>             # Publish the review to GitHub
```

### Comments

```bash
cr comment add <review-id> --file <path> --line <n> --side LEFT|RIGHT --body <text>
cr comment add-file <review-id> --file <path> --body <text>   # Whole-file comment
cr comment update <comment-id> --body <text>
cr comment delete <comment-id>
cr comment resolve <comment-id>
cr comment unresolve <comment-id>
```

### Export / Import

```bash
cr export <review-id> [--format md|json] [-o <path>]   # stdout or file
cr import <path>                                        # Import a JSON review
```

### Repositories

```bash
cr repo list
cr repo add <path>
cr repo remove <id>
```

### Inbox

```bash
cr inbox list
cr inbox refresh
```

## Database location

The CLI shares the desktop app's database. By default:

| OS | Path |
|---|---|
| macOS | `~/Library/Application Support/com.codereview.app/codereview.db` |
| Linux | `$XDG_DATA_HOME/com.codereview.app/codereview.db` |
| Windows | `%APPDATA%/com.codereview.app/codereview.db` |

Override with `--db <path>` or the `CODEREVIEW_DB` environment variable.
