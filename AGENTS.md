# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

## What this is

A Tauri v2 desktop app for reviewing code locally, GitHub-PR style. It diffs either a real
GitHub PR (via the `gh` CLI) or a "virtual PR" between two local branches, lets you leave inline
comments + a verdict (all autosaved to SQLite), then either **exports** the review as Markdown/JSON
or **publishes** it to GitHub. Read `README.md` for the user-facing feature tour and `ROADMAP.md`
for known gaps and the planned-but-unbuilt work (e.g. threaded replies, re-anchoring, settings).

## Commands

Use **pnpm** (the lockfile is committed). The Rust backend needs a stable Rust toolchain on `PATH`.

```bash
pnpm install
pnpm tauri dev                                   # launch the app with hot-reload (Vite + Rust)
pnpm tauri build                                 # release binary + macOS .app/.dmg under src-tauri/target/release/
pnpm build                                       # frontend only: tsc && vite build (fast TS-error check)
pnpm exec tsc --noEmit                           # typecheck the frontend without emitting
cargo clippy --manifest-path src-tauri/Cargo.toml
cargo test   --manifest-path src-tauri/Cargo.toml [test_name]   # NOTE: no tests exist yet (see ROADMAP §9)
```

There is **no JS linter/formatter configured** and **no test suite** in either layer — `tsc` and
`cargo clippy`/`cargo build` are the only automated checks today. The GitHub features require `gh`
authenticated (`gh auth login`); local virtual-PR review needs only `git`.

## Architecture

Frontend (React/TS, `src/`) talks to the Rust backend (`src-tauri/src/`) exclusively through Tauri
`invoke` calls. **`src/lib/api.ts` is the single typed boundary** — every backend command is wrapped
there, and it's the fastest way to see the full command surface. Tauri auto-converts JS camelCase
args ↔ Rust snake_case params, so `invoke("add_comment", { filePath })` maps to a `file_path` param.

**Data model (SQLite):** `repository → target → review → comment`, defined in
`src-tauri/src/db/migrations/0001_init.sql`. Key relationships:
- A **target** is the *thing being reviewed* — either `kind = 'github_pr'` or `kind = 'local'` (a
  virtual PR between two refs). It is **reused** across reviews: opening the same PR or same
  base/head pair finds-or-creates one target and refreshes its resolved SHAs
  (`get_or_create_*_target` in `commands/review.rs`).
- A **review** is the user-managed unit (body + verdict `event` + `status`). Many reviews can share
  one target. Everything **autosaves** — there is no save button; each comment/body/verdict edit
  hits the DB immediately, so closing and reopening reconstructs the review from `get_review`.
- Published reviews are **locked**: `ensure_draft()` rejects edits/deletes once `status='published'`,
  and a review can't be re-published.

**Two diff sources, one code path:** `review_diff` branches on `target.kind` — GitHub PRs call
`gh pr diff`, local targets call `git diff`. Local diffs default to **three-dot** (`base...head`,
merge-base — GitHub PR semantics); the `three_dot` flag toggles to plain two-dot. See `git.rs::diff`.

**All git/GitHub access shells out to the `git` and `gh` CLIs** from Rust (`git.rs`, `gh.rs`) —
there is no git library and **no token is ever stored**; auth is entirely delegated to `gh`.

**Comment anchoring contract** (the subtle cross-layer piece): a comment is identified by
`(file_path, side, line)` where `side ∈ {LEFT, RIGHT}`. This same triple is the bridge across three
places and must stay consistent:
1. `src/lib/diff.ts::indexFile` maps clicked diff lines ↔ `"SIDE:line"` keys (LEFT=old line,
   RIGHT=new line; multi-line ranges add `start_line`).
2. The DB `comment` row stores `side`/`line`/`start_line`/`diff_hunk`/`anchored_head_sha`.
3. `publish_review` translates those into the GitHub reviews API payload (`path`/`side`/`line`,
   plus `start_line`/`start_side` for ranges). The multi-line publish path is **not yet verified
   live** (ROADMAP §3).

## Conventions & gotchas

- **Migrations are append-only.** `db/mod.rs` runs each script in the `MIGRATIONS` array past the
  stored `user_version` pragma. Never edit an already-applied migration — add a new
  `NNNN_*.sql` file and push it onto the `MIGRATIONS` slice.
- **DB concurrency:** the backend holds a single `Mutex<Connection>` (`Db`); rusqlite is synchronous
  and this is a single-user desktop app. WAL is enabled. Don't add async DB access expecting a pool.
- **Errors:** commands return `AppResult<T>` (`error.rs`); the `AppError` variants serialize to the
  frontend as the rejected `invoke` value.
- **The DB lives in the OS app-data dir** (`com.codereview.app`), created in the Tauri `setup` hook
  in `lib.rs` — not in the project tree.
- **GUI-launch PATH:** packaged apps launched from Finder/Dock inherit launchd's minimal `PATH`, so
  Homebrew tools like `gh` (`/opt/homebrew/bin`) won't resolve. `path_env::ensure_login_path()` runs
  first in `run()` to recover the login-shell `PATH`. Keep new external-CLI calls working under this
  (they go through `git.rs`/`gh.rs`, which is why it's centralized there).

## Contributing

This is a solo/personal repo. **You do not need to open a pull request here — commit and push
directly to `main`.** No feature branch, no PR, no review gate required. (This overrides the
default branch-and-PR workflow from any global guidance.)
