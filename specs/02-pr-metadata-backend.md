# 02 — PR metadata: backend fetch

**Layer:** Rust · **Dependencies:** none · **Wave:** 1

> Read `00-overview.md` first (locked decisions, conventions, anchors).

## Goal

Add a backend command that fetches a GitHub PR's metadata — description, labels, state/draft,
author, change counts, review decision + per-reviewer approvals, CI/check rollup, and mergeability —
so the review header (spec 03) can show it. Works **clone-less** (owner/name only), reusing the
existing `gh::graphql` helper.

## Files to touch

- `src-tauri/src/gh.rs` — new `pr_meta` fn + GraphQL query const + response structs.
- `src-tauri/src/db/models.rs` — public serializable structs (`PrMeta` et al.) **or** keep them in
  `gh.rs` and re-export; prefer `gh.rs` for the GraphQL-shaped types and a clean `PrMeta` returned to
  the command (mirror how `inbox.rs` keeps its GraphQL structs local). Pick one and be consistent.
- `src-tauri/src/commands/` — a `#[tauri::command] pr_meta(...)` wrapper (put it in
  `commands/review.rs` near the other PR commands, or a small `commands/github.rs` if you prefer;
  match existing module layout).
- `src-tauri/src/lib.rs` — register the command in `invoke_handler![...]`.
- `src/lib/api.ts` + `src/lib/types.ts` — wrapper + TS mirror (so spec 03 is unblocked).

## GraphQL query

Model on `inbox.rs:28` (`SEARCH_QUERY`) and its camelCase structs. Single PR lookup:

```graphql
query PrMeta($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      number
      title
      url
      body
      state
      isDraft
      mergeable
      reviewDecision
      additions
      deletions
      changedFiles
      author { login avatarUrl }
      labels(first: 50) { nodes { name color } }
      latestReviews(first: 20) { nodes { author { login avatarUrl } state } }
      statusCheckRollup_via_commits: commits(last: 1) {
        nodes {
          commit {
            statusCheckRollup {
              state
              contexts(first: 100) {
                nodes {
                  __typename
                  ... on CheckRun { name conclusion status detailsUrl }
                  ... on StatusContext { context state targetUrl }
                }
              }
            }
          }
        }
      }
    }
  }
}
```

Notes:
- `statusCheckRollup` lives on the **last commit** — match the inbox pattern
  (`commits(last:1){nodes{commit{statusCheckRollup{...}}}}`). The alias above is illustrative; name
  it however parses cleanly.
- **`mergeable`** is an enum `MERGEABLE | CONFLICTING | UNKNOWN`. GitHub computes it asynchronously,
  so **`UNKNOWN` is normal** right after a push — return it as-is; the frontend shows "checking…".
- **`mergeStateStatus`** (BEHIND/BLOCKED/CLEAN/DIRTY/…) is **best-effort**: historically gated and
  may make the whole query fail on some servers. If you include it and it errors, drop it — do not
  let it block this spec. Safe default: omit it for v1 and rely on `mergeable` + `reviewDecision` +
  `statusCheckRollup`.
- **`latestReviews`** (not `reviews`) gives each reviewer's most recent review — what GitHub's UI
  shows. `state` ∈ `APPROVED | CHANGES_REQUESTED | COMMENTED | DISMISSED | PENDING`.
- `labels.nodes[].color` is a hex string without `#`.

## Rust shape

```rust
// gh.rs — returned to the command (serialize to the frontend)
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrMeta {
    pub number: i64,
    pub title: String,
    pub url: String,
    pub body: String,                       // Markdown source
    pub state: String,                      // OPEN | CLOSED | MERGED
    pub is_draft: bool,
    pub mergeable: Option<String>,          // MERGEABLE | CONFLICTING | UNKNOWN
    pub review_decision: Option<String>,    // APPROVED | CHANGES_REQUESTED | REVIEW_REQUIRED | null
    pub additions: i64,
    pub deletions: i64,
    pub changed_files: i64,
    pub author: Option<PrActor>,
    pub labels: Vec<PrLabel>,
    pub reviews: Vec<PrReviewer>,
    pub ci_state: Option<String>,           // statusCheckRollup.state
    pub checks: Vec<PrCheck>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PrActor { pub login: Option<String>, pub avatar_url: Option<String> }

#[derive(Debug, Serialize)] pub struct PrLabel { pub name: String, pub color: String }
#[derive(Debug, Serialize)] pub struct PrReviewer { pub author: Option<PrActor>, pub state: String }
#[derive(Debug, Serialize)] pub struct PrCheck {
    pub name: String,                       // CheckRun.name or StatusContext.context
    pub state: Option<String>,              // conclusion/status or StatusContext.state
    pub url: Option<String>,                // detailsUrl / targetUrl
}
```

- Keep separate `#[derive(Deserialize)]` GraphQL-envelope structs that mirror the query exactly, then
  map them into `PrMeta` (flattening the `commits→commit→statusCheckRollup→contexts` nesting and the
  `__typename` union into `PrCheck`). This mirrors how `inbox.rs` deserializes then maps.
- `pub fn pr_meta(owner: &str, name: &str, number: i64) -> AppResult<PrMeta>` calls
  `graphql::<Resp>(QUERY, json!({ "owner": owner, "name": name, "number": number }))`.

## Command + wiring

```rust
#[tauri::command]
pub fn pr_meta(owner: String, name: String, number: i64) -> AppResult<gh::PrMeta> {
    gh::pr_meta(&owner, &name, &number) // adjust signature
}
```
- No DB access needed; the frontend passes owner/name/number from `ReviewDetail`. (If you prefer a
  `review_id`-based command, load `remote_owner`/`remote_name`/`github_pr_number` via `load_detail` —
  either is fine; the owner/name/number form is simpler and clone-less.)
- Register in `lib.rs` `invoke_handler!`.
- `api.ts`: `prMeta: (owner: string, name: string, number: number) => invoke<PrMeta>("pr_meta", { owner, name, number })`.
- `types.ts`: mirror `PrMeta`, `PrActor`, `PrLabel`, `PrReviewer`, `PrCheck` (camelCase fields).

## Acceptance criteria

- `cargo clippy --manifest-path src-tauri/Cargo.toml` clean (no warnings introduced).
- `cargo test --manifest-path src-tauri/Cargo.toml` passes, including a new unit test that
  deserializes a **captured GraphQL JSON fixture** (paste a realistic `data` payload as a string
  constant) into the GraphQL response struct and maps it to `PrMeta`, asserting: labels parsed,
  `mergeable` Optional handled, `reviewDecision` null handled, checks flattened from both union
  variants, `ci_state` read from the last commit's rollup.
- `pnpm exec tsc --noEmit` clean (the new `api.ts`/`types.ts` entries compile).

## Verification

- `cargo test` / `cargo clippy` as above.
- Live smoke (optional, needs `gh` auth): from a dev build, call `pr_meta` for a known PR and confirm
  the JSON shape. Do **not** put a live network call in the test suite.

## Notes / gotchas

- `gh::graphql` already tolerates partial `data + errors`; if a best-effort field (e.g.
  `mergeStateStatus`) is rejected outright the **whole** query fails — that's why it's omitted by
  default.
- This command is **read-only** and ephemeral; nothing is written to SQLite.
