# Spec set: GitHub integration depth (PR metadata + existing PR threads)

This directory holds small, independently-executable specs that together deliver ROADMAP **§3
"GitHub integration depth"** — specifically **PR metadata** and **showing existing PR threads** —
plus the two §1 prerequisites they need (comment **Markdown rendering** and a **read-only threaded
display**).

Each spec is scoped so an agent with **clean context** can execute it from the spec alone. Read this
overview first, then the individual spec.

## Locked design decisions

- **Core only.** We implement PR metadata + read-only display of existing PR threads. We do **not**
  build "reply/resolve GitHub threads" or §2 "robust re-anchoring" here (see Future follow-ups).
- **Read-only thread display.** No local reply authoring. **Consequence:** `publish_review` /
  `build_publish_payload` (`commands/review.rs`) and `export.rs` are **left untouched** — local
  comments never gain replies, and fetched GitHub threads are read-only and never published/exported.
- **Markdown lib: `react-markdown` + `remark-gfm`.** React-native, **sanitized by default** (do not
  add `rehype-raw`). GFM gives tables/task-lists/strikethrough/autolinks.
- **Existing PR threads are fetched ephemerally** via React Query — **not persisted** into the
  `comment` table. They render read-only and visually "from GitHub", distinct from local drafts.
- **Outdated / un-anchorable threads reuse the orphan fallback.** A thread whose line isn't in the
  current `gh pr diff` renders in a distinct "from GitHub" orphan sub-block.

## Architecture anchors (verified — true at spec authoring time)

- **`gh::graphql<T>(query, variables) -> AppResult<T>`** — `src-tauri/src/gh.rs:101`. Runs
  `gh api graphql --input -`, cwd-independent, tolerates partial `data`+`errors`. **Reuse it** for
  all new GraphQL. Reference pattern: `src-tauri/src/inbox.rs:28` (`SEARCH_QUERY`) and its
  `#[serde(rename_all = "camelCase")]` response structs at `inbox.rs:98`.
- **`load_detail` → `ReviewDetail`** — `commands/review.rs:378`. Already carries `target.kind`,
  `target.github_pr_number`, `remote_owner`, `remote_name`, `target.head_sha`.
- **Command wiring:** functions in `gh.rs`; models in `src-tauri/src/db/models.rs`;
  `#[tauri::command]` wrappers in `src-tauri/src/commands/*.rs`; **register in
  `src-tauri/src/lib.rs` `invoke_handler![...]`**; wrap in `src/lib/api.ts`; mirror TS types in
  `src/lib/types.ts`.
- **Anchoring contract:** `src/lib/diff.ts::indexFile` returns `keyByAnchor` keyed `"SIDE:line"`
  (RIGHT = new/head line, LEFT = old/base line). `FileReview` (`src/components/ReviewView.tsx:482`)
  and `FileViewPane.tsx:96` group comments by `keyByAnchor.get(`${side}:${line}`)`; misses go to an
  `orphans` array rendered in an `.orphan-comments` block (`ReviewView.tsx:745`).
- **`CommentItem`** — `src/components/ReviewView.tsx:883`. Renders body in a `<textarea>`; no
  Markdown today. **`ReviewHeader`** — `ReviewView.tsx:143`. Where the PR metadata panel goes.
- No Markdown lib installed (`package.json`: `react-diff-view`, `refractor`). **Vitest** is
  configured (`vitest.config.ts`); pure helpers have tests (`src/lib/diff.test.ts`) — match that style.

## Shared conventions (apply to every spec)

- **Adding a Rust command:** add the fn in `gh.rs` (uses `gh::graphql`), the struct(s) in
  `models.rs` with `#[serde(rename_all = "camelCase")]` and `Option<_>` for nullables, a
  `#[tauri::command]` wrapper, **a line in `lib.rs` `invoke_handler!`**, the `api.ts` wrapper, and
  the `types.ts` mirror. Camel↔snake is auto-converted by Tauri.
- **Never add a new `gh`/`git` shell path outside `gh.rs`/`git.rs`** (GUI-launch PATH handling lives
  there — see project `CLAUDE.md`).
- **Verification:** backend → `cargo test --manifest-path src-tauri/Cargo.toml` and
  `cargo clippy --manifest-path src-tauri/Cargo.toml`. Frontend → `pnpm exec tsc --noEmit` and
  `pnpm test`. There is **no JS linter/formatter** — don't invent one.
- Errors are `AppResult<T>` / `AppError` (`error.rs`); they serialize to the frontend as the
  rejected `invoke` value.
- This is a solo repo: **commit directly to `main`**, no PR (project `CLAUDE.md`).

## The specs

| # | File | Layer | Scope | Deps |
|---|------|-------|-------|------|
| 01 | `01-comment-markdown.md` | Frontend | `<Markdown>` component; render comment bodies (Write/Preview toggle) | — |
| 02 | `02-pr-metadata-backend.md` | Rust | `pr_meta` GraphQL command + models | — |
| 03 | `03-pr-metadata-frontend.md` | Frontend | `PrMetaPanel` in the review header | 01, 02 |
| 04 | `04-pr-threads-backend.md` | Rust | `pr_review_threads` GraphQL command (paginated) + models | — (run after 02) |
| 05 | `05-pr-threads-frontend.md` | Frontend | Read-only `GithubThread` rendering, anchored to the diff | 01, 04 |

## Dependency graph & execution waves

```
01 (md, FE) ─┬─────────────► 03 (pr-meta FE)
02 (pr-meta RS) ─────────────┘
                              05 (pr-threads FE)
01 ──────────────────────────┘
04 (pr-threads RS) ──────────┘     (04 soft-after 02: shares gh.rs/models.rs/lib.rs/api.ts/types.ts)
```

- **Wave 1 (parallel):** `01` (frontend) ‖ `02` (Rust). Disjoint file sets.
- **Wave 2 (parallel):** `03` (frontend; needs 01+02) ‖ `04` (Rust; run **after** 02 lands to avoid
  textual conflicts in the shared backend registration files).
- **Wave 3:** `05` (frontend; needs 01+04).

The three frontend specs touch **different functions** of `ReviewView.tsx` (`CommentItem` /
`ReviewHeader` / `FileReview`+orphans) and run in different waves, so they don't collide. The two
Rust specs share registration files → keep them sequential (02 then 04).

## End-to-end verification (after the whole set lands)

1. `pnpm exec tsc --noEmit` && `pnpm test` clean; `cargo test` && `cargo clippy` clean.
2. `pnpm tauri dev`, open a **GitHub PR** review (via the inbox or a repo's PR list):
   - Header shows the PR description/labels/CI/mergeability/approvals.
   - Existing review threads render inline on their lines (read-only, "from GitHub"), with
     resolved/outdated handling; outdated/un-anchorable threads appear in the GitHub-orphan sub-block.
   - Comment bodies (local drafts **and** GitHub threads) render as Markdown.
3. Open a **local virtual-PR** review: behaves exactly as before — no PR panel, no threads,
   but local comment bodies now render Markdown.

## Future follow-ups (explicitly out of scope here)

- **Reply to / resolve existing GitHub threads** via API (§3). Spec 04 already fetches each
  comment's `databaseId` and the thread `id` to make this cheap later.
- **Robust comment re-anchoring** (§2) — would replace the orphan fallback for outdated threads.
- **Local threaded replies** (§1). If ever added, `publish_review` / `build_publish_payload` and
  `export.rs::render_markdown` must become reply-aware (group by root, fold replies into the root).
