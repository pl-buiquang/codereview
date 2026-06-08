# 04 — Existing PR threads: backend fetch

**Layer:** Rust · **Dependencies:** none (run **after** spec 02 — shared backend files) · **Wave:** 2

> Read `00-overview.md` first (locked decisions, conventions, anchors).

## Goal

Add a backend command that fetches a GitHub PR's existing **review threads** (the inline
comment threads on the diff) so the frontend (spec 05) can render them read-only alongside local
drafts. Works clone-less, reusing `gh::graphql`. **Ephemeral** — nothing is persisted.

## Ordering

Run **after spec 02 lands.** Both edit `gh.rs`, `db/models.rs`, `lib.rs`, `api.ts`, `types.ts`;
sequential execution avoids textual merge conflicts in those shared registration points.

## Files to touch

- `src-tauri/src/gh.rs` — new `pr_review_threads` fn + GraphQL query const + response structs.
- `src-tauri/src/db/models.rs` **or** `gh.rs` — `PrThread` / `PrThreadComment` (be consistent with
  where spec 02 put `PrMeta`).
- `src-tauri/src/commands/` — `#[tauri::command] pr_review_threads(...)` wrapper (next to `pr_meta`).
- `src-tauri/src/lib.rs` — register in `invoke_handler![...]`.
- `src/lib/api.ts` + `src/lib/types.ts` — wrapper + TS mirror.

## GraphQL query

```graphql
query PrThreads($owner: String!, $name: String!, $number: Int!, $cursor: String) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      reviewThreads(first: 50, after: $cursor) {
        pageInfo { hasNextPage endCursor }
        nodes {
          id
          isResolved
          isOutdated
          isCollapsed
          path
          line
          startLine
          originalLine
          diffSide
          startDiffSide
          subjectType
          comments(first: 100) {
            pageInfo { hasNextPage endCursor }
            nodes {
              id
              databaseId
              author { login avatarUrl }
              body
              createdAt
              url
              diffHunk
              outdated
            }
          }
        }
      }
    }
  }
}
```

Notes:
- **Pagination is mandatory** for `reviewThreads`: loop, passing `endCursor` as `$cursor`, until
  `hasNextPage` is false. The default page truncates large PRs silently.
- Use the comment **`body`** (Markdown source) — **not** `bodyText` — because spec 05 renders it via
  `<Markdown>`.
- **`databaseId`** on each comment and the thread **`id`** are fetched now (cheap) so a future
  reply/resolve feature has the keys it needs.
- **`subjectType`** (`LINE | FILE`) is a newer schema field — **best-effort**. If it risks query
  rejection on older GitHub Enterprise schemas, drop it and treat `line == null` as the "file-level
  thread" signal instead. `gh::graphql` tolerates partial `data+errors`, but a rejected field can
  fail the whole query, so prefer dropping it if unsure.
- `comments(first: 100)` per thread is almost always enough; if a thread's `comments.pageInfo
  .hasNextPage` is true, it's fine for v1 to take the first 100 — but **note the cap in a code
  comment** (don't silently drop). (Paginating per-thread comments too is optional polish.)
- `line`/`startLine`/`startDiffSide`/`path` can be null (outdated/file-level threads) → `Option<_>`.
- `diffSide` ∈ `LEFT | RIGHT`. `line` is the position on that side in the current diff.

## Rust shape

```rust
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrThread {
    pub id: String,
    pub is_resolved: bool,
    pub is_outdated: bool,
    pub is_collapsed: bool,
    pub path: Option<String>,
    pub line: Option<i64>,
    pub start_line: Option<i64>,
    pub original_line: Option<i64>,
    pub diff_side: Option<String>,      // LEFT | RIGHT
    pub start_diff_side: Option<String>,
    pub subject_type: Option<String>,   // LINE | FILE (best-effort)
    pub comments: Vec<PrThreadComment>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrThreadComment {
    pub id: String,
    pub database_id: Option<i64>,
    pub author: Option<PrActor>,        // reuse PrActor from spec 02
    pub body: String,                   // Markdown source
    pub created_at: String,
    pub url: String,
    pub diff_hunk: Option<String>,
    pub outdated: bool,
}
```

- Deserialize into query-shaped envelope structs, then flatten into `Vec<PrThread>`. Mirror the
  inbox deserialize-then-map approach.
- `pub fn pr_review_threads(owner: &str, name: &str, number: i64) -> AppResult<Vec<PrThread>>` —
  paginate internally, concatenating `nodes` across pages.

## Command + wiring

```rust
#[tauri::command]
pub fn pr_review_threads(owner: String, name: String, number: i64)
    -> AppResult<Vec<gh::PrThread>> { gh::pr_review_threads(&owner, &name, number) }
```
- Register in `lib.rs` `invoke_handler!`.
- `api.ts`: `prReviewThreads: (owner, name, number) => invoke<PrThread[]>("pr_review_threads", { owner, name, number })`.
- `types.ts`: mirror `PrThread` + `PrThreadComment` (reuse the `PrActor` type from spec 02).

## Acceptance criteria

- `cargo clippy --manifest-path src-tauri/Cargo.toml` clean.
- `cargo test --manifest-path src-tauri/Cargo.toml` passes, including a unit test that deserializes a
  **captured `reviewThreads` JSON fixture** and maps it to `Vec<PrThread>`, asserting: a resolved
  thread, an outdated thread with `line == null`, a multi-comment thread (root + reply), and
  `diffSide`/`line` preserved. (Pagination logic can be unit-tested by feeding a two-page fixture if
  you factor the page-merge into a pure helper; otherwise assert single-page mapping.)
- `pnpm exec tsc --noEmit` clean.

## Verification

- `cargo test` / `cargo clippy`.
- Live smoke (optional, needs `gh` auth): call `pr_review_threads` for a PR that has inline
  review comments; confirm thread/comment counts and resolved/outdated flags. No live call in tests.

## Notes / gotchas

- Read-only and ephemeral — **no SQLite writes**, no `comment`-table involvement.
- Don't fold replies or transform bodies here — return raw threads; spec 05 handles presentation.
