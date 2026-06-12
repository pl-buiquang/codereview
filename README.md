# codereview

A cross-platform **desktop app for reviewing code locally**, GitHub-PR style. Point it at a
local git repository and review **open GitHub PRs** (via the `gh` CLI) *or* a **"virtual PR"**
between any two local branches. Leave inline comments and a verdict like on GitHub, stored in
a local database, then either **publish the review to GitHub** or **export it as Markdown/JSON**
for an AI agent to read.

Built with **Tauri v2** (Rust) + **React/TypeScript**.

---

## Features

- **Local-first** — works against any local git repo; no server.
- **Two diff sources, both first-class**
  - *Virtual PR*: pick any `base` and `compare` branch; diff uses the merge-base (`base...head`,
    GitHub semantics) with a two-dot toggle.
  - *GitHub PR*: list open PRs and fetch their diff through `gh`.
- **GitHub-style diff viewer** — split/unified, syntax highlighting, +/− counts, per-file
  "viewed" collapse.
- **Inline review comments** — click a line to comment; **shift-click** another line on the
  same side to comment on a **multi-line range**.
- **Keyboard shortcuts** — `]`/`[` next/previous file, `n`/`p` next/previous comment thread,
  `j`/`k` move a line cursor, `c` comment on the focused line, `?` for the full list.
- **Reviews are a first-class, autosaved model**
  - Every comment / summary / verdict change persists immediately (no save button).
  - Close the app and reopen — a review is fully reconstructed.
  - Many reviews per target; resume or delete any from the Reviews list.
- **Two outputs**
  - **Export** Markdown (AI-readable: file + line + diff hunk + comment) or JSON. Repeatable.
  - **Publish** a GitHub-PR review (comment / approve / request-changes) via `gh`. A published
    review is locked (can't edit or re-publish) but can still be exported.

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
password.) The bundles are **not** OS code-signed (no Apple/Windows certificate yet) — see
`docs/signing.md`. `pnpm tauri dev` / `ubt start` don't build updater artifacts, so they need none
of this.

---

## Usage

1. **Add a repository** — click **+ Add repo** and choose a local git repo.
2. Pick it in the sidebar, then choose a tab:
   - **Virtual PR** — select `base` and `compare` branches → **Preview diff** to look, or
     **Start review** to begin one.
   - **GitHub PRs** — lists open PRs (requires `gh auth`); click **Review** to start one.
3. In a review: **click a line** to comment (shift-click to select a range), fill in the
   **summary** and **verdict**. Everything autosaves.
4. Finish with **Export** (Markdown/JSON, any review, repeatable) or **Publish** (GitHub PR
   reviews only; locks the review).

The Reviews list shows every saved review for the repo — open to resume, or ✕ to delete.

---

## Architecture

```
src-tauri/                 # Rust backend
  src/
    lib.rs                 # Tauri builder, DB init, command registration
    db/                    # rusqlite connection + migrations + row models
    git.rs                 # git CLI: branches, rev-parse, diff
    gh.rs                  # gh CLI: PR list/view/diff, post review
    export.rs              # review model -> Markdown / JSON
    commands/              # #[tauri::command] handlers (repo, review, gh, export)
src/                       # React frontend
  components/              # RepoSidebar, RepoView, ReviewView, DiffViewer
  lib/                     # typed invoke wrappers, diff helpers
  store.ts                 # Zustand UI state
```

**Data model** (SQLite): `repository → target (local | github_pr) → review → comment`.
A `target` is reused across multiple reviews. The database lives in the OS app-data directory
(e.g. `~/.local/share/com.codereview.app/codereview.db` on Linux).
