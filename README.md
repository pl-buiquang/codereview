# codereview

A cross-platform **desktop app for reviewing code locally**, GitHub-PR style. Point it at a
local git repository, browse its history in a **commit graph**, and review **open GitHub PRs**
(via the `gh` CLI) *or* a **"virtual PR"** between any two local branches. Leave inline comments and a verdict like on GitHub, stored in
a local database, then either **publish the review to GitHub** or **export it as Markdown/JSON**
for an AI agent to read.

Built with **Tauri v2** (Rust) + **React/TypeScript**.

---

## Features

Each area has a page under [`docs/features/`](docs/features/).

- **Local-first** — works against any local git repo; no server. GitHub access is delegated to `gh`.
- **Home** — a sidebar with Sessions, Board, Inbox, Reviews, Worktrees and Repositories, plus your
  open repositories (they stay loaded inside the Home tab). Fold it to an icon rail. A command
  palette (`⌘P`) jumps anywhere.
  → [sessions & worktrees](docs/features/sessions-worktrees.md)
- **Git graph** — a GitKraken-style commit graph with branches, worktrees, tags, a `// WIP` row and
  light git actions (checkout, branch/tag, pull, push, stage, commit, stash).
  → [graph](docs/features/graph.md)
- **Review** — GitHub-style split/unified diff with syntax and word-level highlighting, inline,
  multi-line and whole-file comments, suggested changes, threaded replies, resolve, outdated-comment
  re-anchoring, and a keyboard-driven workflow. Everything autosaves to SQLite.
  → [review](docs/features/review.md)
- **Image diffs** — before / after / pixel diff of the PNG, JPEG and SVG files a review changes,
  generated in the background into a disk cache, with an optional AI report that describes each
  change with close-ups. → [image diffs](docs/features/image-diffs.md)
- **Two diff sources, one flow** — a real GitHub PR (via `gh`) or a *virtual PR* between any two
  local branches (three-dot by default, two-dot toggle).
  → [GitHub](docs/features/github.md)
- **Publish or export** — publish to GitHub (comment / approve / request-changes, or as a pending
  draft you submit later), or export Markdown/JSON for an agent; import JSON back.
  → [GitHub](docs/features/github.md), [import & export](docs/features/import-export.md)
- **Inbox & Board** — a triage queue for what GitHub says needs you, and a kanban of work in flight
  with an auto-merge monitor and card metadata (plans, links, Claude sessions).
  → [board & inbox](docs/features/board-inbox.md)
- **Focus & AI** — AI-tiered hunks so you read what matters, in-app review runs, a chat panel and
  editable prompts, via the `claude` CLI.
  → [focus & AI](docs/features/focus-and-ai.md)
- **Routines** — unattended `claude -p` runs on PRs, issues and the board: babysit CI, plan an issue,
  refine the plan in an agent session, implement it into a draft PR.
  → [routines](docs/features/routines.md)
- **`cr` CLI and deep links** — script the same database from a terminal; `codereview://` links.
  → [CLI](docs/features/cli.md)
- **Settings & themes** — 4 design directions × dark/light/system, sync modes, AI models and
  usage, backup, auto-update. → [settings](docs/features/settings.md), [THEMING.md](THEMING.md)

---

## Tech stack

| Layer | Choice |
|---|---|
| Shell | Tauri v2 |
| Frontend | React + TypeScript + Vite |
| Backend | Rust (Tauri commands) |
| Storage | SQLite via `rusqlite` (versioned migrations) |
| Diff UI | `react-diff-view` + `refractor` (syntax highlight) |
| Async / state | TanStack Query + Zustand |
| Git / GitHub | the `git` and `gh` CLIs, invoked from Rust |

The app never stores GitHub tokens — authentication is delegated entirely to `gh`.

---

## Prerequisites

- **Rust** (stable) + Cargo
- **Node** ≥ 18 and **pnpm**
- **git** on `PATH`
- **gh** (GitHub CLI), authenticated with `gh auth login` — only needed for the GitHub PR
  features
- Linux only: the usual Tauri system deps (`webkit2gtk-4.1`, `libgtk-3`, `librsvg2`,
  `libayatana-appindicator3`, etc.). See https://tauri.app/start/prerequisites/.

---

## Develop

```bash
pnpm install
pnpm tauri dev
```

This starts Vite and launches the desktop window with hot-reload for the frontend and
auto-rebuild for the Rust backend.

Useful checks:

```bash
pnpm exec tsc --noEmit          # typecheck the frontend
pnpm test                       # frontend tests (vitest)
cargo clippy --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml   # Rust tests
```

> Prefer universal verbs? The committed `ubt.toml` maps them: `ubt start` (= `pnpm tauri dev`),
> `ubt build` (= `pnpm tauri build`), `ubt check` (tsc), `ubt test` (vitest), `ubt lint` (clippy).

---

## Build the final artifact

```bash
pnpm tauri build
```

This compiles an optimized release binary and packages platform installers. Outputs land in
`src-tauri/target/release/`:

- **Standalone binary**: `src-tauri/target/release/codereview` (`.exe` on Windows)
- **Bundled installers**: `src-tauri/target/release/bundle/`
  - **Linux** → `deb/`, `rpm/`, `appimage/` (e.g. `bundle/appimage/CodeReview_0.1.0_amd64.AppImage`)
  - **macOS** → `dmg/`, `macos/CodeReview.app`
  - **Windows** → `nsis/` and/or `msi/`

Run the standalone binary directly:

```bash
./src-tauri/target/release/codereview
```

Or, on Linux, the portable AppImage:

```bash
chmod +x src-tauri/target/release/bundle/appimage/CodeReview_*_amd64.AppImage
./src-tauri/target/release/bundle/appimage/CodeReview_*_amd64.AppImage
```

> To control which installers are produced, edit `bundle.targets` in
> `src-tauri/tauri.conf.json` (currently `"all"`).

**Signing.** `bundle.createUpdaterArtifacts` is on, so `pnpm tauri build` (and `ubt build`) sign
the update bundle with your minisign key and **fail without it**. Provide the key first:

```bash
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/codereview.key)"
read -rs "TAURI_SIGNING_PRIVATE_KEY_PASSWORD?Updater key password: " && export TAURI_SIGNING_PRIVATE_KEY_PASSWORD; echo
pnpm tauri build
```

(`read -rs` keeps the password out of your shell history; drop that line if your key has no
password.) To avoid retyping it, keep both variables in a `chmod 600` env file, `~/.tauri/codereview.env`
(`TAURI_SIGNING_PRIVATE_KEY=…` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD=…`), and build with
`set -a; source ~/.tauri/codereview.env; set +a; pnpm tauri build`. Tauri only reads
`TAURI_SIGNING_PRIVATE_KEY` (the key *contents*); `TAURI_SIGNING_PRIVATE_KEY_PATH` is ignored. The bundles are **not** OS code-signed (no Apple/Windows certificate yet) — see
`docs/signing.md`. `pnpm tauri dev` / `ubt start` don't build updater artifacts, so they need none
of this.

---

## Quick start

1. **Add a repository** — click **+ Add repo** and choose a local git repo, then open it from
   **Repositories**.
2. Browse history on the **Graph** tab, or open the **GitHub PRs** tab and click **Review**. To
   review local work, right-click a branch in the graph and choose **Review vs `<default>`**.
3. **Click a line** to comment (shift-click for a range), write a summary and pick a verdict.
   Everything autosaves; close and reopen at any time.
4. Finish with **Export** or **Publish**.

---

## Architecture

```
src-tauri/                 # Rust backend
  src/
    lib.rs                 # Tauri builder, DB init, command registration
    db/                    # rusqlite connection, append-only migrations, row models
    mirror/                # disposable GitHub mirror (gh_pr, gh_issue, …) — ADR 0002
    sync/                  # backend-owned sync engine: pipeline, scheduler — ADR 0003
    graph/                 # commit-graph layout and git mutations
    git.rs, gh.rs          # git / gh CLI wrappers (no git library, no stored token)
    provider.rs            # forge-provider trait over gh.rs
    anchor.rs              # comment line re-anchoring through diffs
    diff_cache.rs          # LRU of parsed diffs per (repo, PR, base, head)
    image_diff/            # pixel diffs of changed images: engine, background jobs, disk cache
    inbox.rs, board.rs     # inbox / kanban pipelines; board_monitor.rs = auto-merge monitor
    claude_sessions.rs     # Claude Code session discovery (+ terminal/, session_summaries.rs)
    worktree.rs, worktree_gc.rs   # worktree helpers and the janitor
    watch.rs, repo_sync.rs # lazy file watchers, background fetch — ADR 0004
    export.rs              # review model -> Markdown / JSON
    prompts.rs, ai_usage.rs# editable AI prompts, usage tracking
    deep_link.rs           # codereview:// handling
    commands/              # #[tauri::command] handlers, one module per area
    bin/cr.rs              # `cr` CLI, shares the same SQLite db and modules
src/                       # React frontend
  components/              # views (Review, Board, Inbox, Sessions, Settings, …); graph/ subfolder
  lib/                     # api.ts (the typed invoke boundary), diff helpers, settings, themes
  store.ts                 # Zustand UI state
docs/                      # features/, adr/, signing.md
skills/                    # Claude Code skills (local-review, board)
```

**Data model** (SQLite): `repository → target (local | github_pr) → review → comment`. A `target`
is reused across reviews; each review pins its own base/head SHAs ([ADR 0001](docs/adr/0001-pin-lives-on-review.md)).
GitHub facts live in a separate disposable mirror. The database lives in the OS app-data directory
(e.g. `~/.local/share/com.codereview.app/codereview.db` on Linux) — `pnpm tauri dev` uses its own
identifier (`com.codereview.app.dev`) and therefore its own empty database, so local development
never touches your real data.

More: [`docs/adr/`](docs/adr/) (design decisions), [`GLOSSARY.md`](GLOSSARY.md) (domain terms),
[`THEMING.md`](THEMING.md), [`docs/signing.md`](docs/signing.md).
