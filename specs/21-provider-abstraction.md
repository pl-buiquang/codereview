# Spec 21 — Provider trait abstraction over `gh.rs`

Implements ROADMAP §3 "**Provider abstraction** — factor `gh.rs` behind a trait so
GitLab/Bitbucket/Gitea could be added later" (`ROADMAP.md:47-48`).

> **Sequencing: implement LAST (wave 5), after specs 10, 17, 18 and 19 have merged.** This spec
> wraps the GitHub surface *as it actually exists* at implementation time. The signatures below
> were transcribed from those specs before implementation; **the merged code wins** — Task 1 is a
> mandatory reconciliation pass.

## Problem

Every GitHub interaction is a hard-wired `gh::` call. The functions themselves are cleanly
centralized in `src-tauri/src/gh.rs` (per CLAUDE.md), but the *call sites* name the GitHub module
directly, so supporting a second forge would mean an `if host == …` at every one of them:

- `src-tauri/src/commands/review.rs`: `gh::pr_view` in `refresh_target_shas` (`review.rs:226`)
  and `create_review_for_pr` (`review.rs:460`), `gh::compare` in `reanchor_review_comments`
  (`review.rs:341`), `gh::pr_diff` in `review_diff` (`review.rs:479`), `gh::file_at_ref` in
  `file_source` (`review.rs:533`), `gh::post_review` in `publish_review` (`review.rs:876`).
- `src-tauri/src/commands/gh.rs`: `gh::auth_status` (`commands/gh.rs:11`, `:53`), `gh::list_prs`
  (`:16`), `gh::pr_meta` (`:25`), `gh::pr_review_threads` (`:36`).
- Specs 10/17/18/19 add more direct calls in the same two files: `gh::merge_base_sha`
  (spec 10 — `create_review_for_pr`, `refresh_target_shas`, `file_source` backfill),
  `gh::review_comments` (spec 17 — `capture_github_comment_ids`), `gh::reply_to_thread` /
  `gh::set_thread_resolved` (spec 18 — two new commands in `commands/gh.rs`),
  `gh::submit_pending_review` / `gh::delete_pending_review` (spec 19 — pending commands).

There is no seam: a `GitLabProvider` has nowhere to plug in. (Line anchors above are pre-wave-5;
specs 10/17/18/19 shift them — locate by symbol name.)

## Decisions (locked)

- **Pure indirection, ZERO behavior change.** Function bodies stay in `gh.rs` untouched;
  `GithubProvider` is a unit struct whose every method is a one-line delegation. `git.rs` stays
  concrete (local git is not a forge). No frontend, DB, or `gh.rs` change of any kind. The
  existing test suite is the regression net.
- **The trait is fully synchronous.** Verified: every public `gh.rs` function is a blocking
  subprocess wrapper (`run_gh`/`run_gh_stdin`, `gh.rs:45-86`); the `async` in `commands/gh.rs`
  (`pr_meta` `:23-26`, `pr_review_threads` `:30-37`, spec 18's two commands) exists only to keep
  the call off the UI thread — those command fns stay `async` with sync bodies, unchanged. No
  `async_trait`, no futures in the trait.
- **Dynamic dispatch (`dyn`), not generics.** Provider choice is runtime data (the repo's host,
  once a second forge exists), and `#[tauri::command]` fns registered in `generate_handler!`
  (`lib.rs:44-86`) need concrete signatures — threading a `<P: ReviewProvider>` type parameter
  through the command layer is impossible without monomorphizing the whole command set. vtable
  cost is noise next to spawning a CLI subprocess. The trait must therefore stay **object-safe**:
  every method takes `&self`, no generic methods, concrete return types.
- **Factory returns `&'static dyn ReviewProvider`, not `Box<dyn>`.** Providers are stateless unit
  structs (all state lives in the CLI's own auth/config), so a static instance suffices — no
  allocation, no lifetime plumbing. If a future provider ever needs per-repo state, switching the
  factory to `Box<dyn ReviewProvider>` is a mechanical return-type change.
- **`provider_for()` takes no argument today** (deliberate deviation from the obvious
  `provider_for(repo)`): nothing in the schema or app encodes a host yet, and several call sites
  (`gh_auth_status`, the owner/name-addressed `pr_meta`/`pr_review_threads` commands) have no
  repository in scope at all — a hint parameter the body ignores would be a lie at half the call
  sites. When a second forge lands, the factory gains the real discriminator (see the extension
  recipe) and the compiler enumerates every call site. The factory exists *now* purely so the call
  sites already go through one named seam.
- **The trait reuses today's `gh.rs` types verbatim**: `GhRepo`, `PrInfo`, `PrSummary`, `PrMeta`,
  `PrThread`, `ComparedFile`, `ReviewComment`, and `post_review`'s GitHub-REST `payload_json:
  &str` contract. Neutralizing these (renames, a structured publish payload) is real design work
  that belongs to the spec that adds the second provider — doing it now would violate "zero
  behavior change" and churn every test. The known GitHub leaks are catalogued in the extension
  recipe instead.
- **`gh::graphql<T>` stays OUT of the trait.** It is generic (`gh.rs:101`), hence not
  object-safe, and it is GitHub *transport*, not forge *capability* — providers use whatever
  transport they like internally.
- **`inbox.rs` stays concrete GitHub.** It consumes raw GraphQL search queries
  (`inbox.rs:492/523/532/597`) and `gh::pr_files` (`inbox.rs:560`) — the inbox is a GitHub-only
  feature until someone designs a cross-forge inbox. Likewise `check_environment`'s `ToolEnv`
  keeps reporting the `gh` binary specifically; only its `gh_authed` flag routes through the
  provider (it maps 1:1 to `auth_status`).
- **No speculative trait surface.** One method per `gh.rs` function *actually called from
  `commands/` at implementation time*. If a prerequisite spec was dropped or renamed a function,
  the trait mirrors reality — drop/rename the method, don't stub it. (The assignment sheet's
  "resolve_thread/unresolve_thread" is, per merged spec 18, a single
  `set_thread_resolved(thread_id, resolved)` — follow the code.)
- **`fn name(&self) -> &'static str` is part of the trait.** One line per provider; gives logs a
  stable tag and gives the factory smoke test something real to assert.

## Design

```
commands/review.rs ─┐                       ┌─ gh.rs (bodies unchanged: run_gh, graphql, …)
commands/gh.rs ─────┤→ provider::provider_for() → &'static GithubProvider ──┘
                    │        (the seam)            [future: GitlabProvider → glab CLI]
inbox.rs ───────────┴──────────────────────────→ gh.rs directly (out of scope, GitHub-only)
git.rs — concrete, untouched
```

### 1. NEW `src-tauri/src/provider.rs`

```rust
//! Forge-provider seam (ROADMAP §3). One trait covering exactly the GitHub
//! surface the command layer uses; `GithubProvider` delegates 1:1 to `gh.rs`.
//! See the "Adding a provider" recipe at the bottom of this file.

use crate::error::AppResult;
use crate::gh::{self, ComparedFile, GhRepo, PrInfo, PrMeta, PrSummary, PrThread, ReviewComment};

/// Everything the app asks of a code-review forge. Object-safe and fully
/// synchronous: implementations shell out to a CLI; `async` lives only at the
/// Tauri command layer. Exactly one method per `gh.rs` entry point called from
/// `commands/` — no speculative surface.
///
/// v1 deliberately reuses `gh.rs` types (`GhRepo`, the REST-shaped DTOs, the
/// JSON-string publish payload). They are part of the trait contract until a
/// second provider forces neutral types — see the recipe below.
pub trait ReviewProvider: Send + Sync {
    /// Stable short tag for logs/tests ("github").
    fn name(&self) -> &'static str;

    // --- auth ---------------------------------------------------------------
    fn auth_status(&self) -> bool;

    // --- PR reads -----------------------------------------------------------
    fn list_prs(&self, ctx: &GhRepo) -> AppResult<Vec<PrSummary>>;
    fn pr_view(&self, ctx: &GhRepo, number: i64) -> AppResult<PrInfo>;
    fn pr_diff(&self, ctx: &GhRepo, number: i64) -> AppResult<String>;
    fn pr_meta(&self, owner: &str, name: &str, number: i64) -> AppResult<PrMeta>;
    fn pr_review_threads(&self, owner: &str, name: &str, number: i64)
        -> AppResult<Vec<PrThread>>;
    fn compare(&self, owner: &str, name: &str, base: &str, head: &str)
        -> AppResult<Vec<ComparedFile>>;
    fn merge_base_sha(&self, owner: &str, name: &str, base: &str, head: &str)
        -> AppResult<String>;                                              // spec 10
    fn file_at_ref(&self, owner: &str, name: &str, file_path: &str, git_ref: &str)
        -> AppResult<String>;

    // --- review writes ------------------------------------------------------
    /// `payload_json` is the GitHub REST reviews payload built by
    /// `build_publish_payload` — a v1 leak, kept verbatim (zero behavior change).
    fn post_review(&self, owner: &str, name: &str, number: i64, payload_json: &str)
        -> AppResult<i64>;
    fn review_comments(&self, owner: &str, name: &str, number: i64, review_id: i64)
        -> AppResult<Vec<ReviewComment>>;                                  // spec 17
    /// `event` ∈ APPROVE | REQUEST_CHANGES | COMMENT (GitHub's vocabulary — v1 leak).
    fn submit_pending_review(&self, owner: &str, name: &str, number: i64,
        review_id: i64, event: &str) -> AppResult<()>;                     // spec 19
    fn delete_pending_review(&self, owner: &str, name: &str, number: i64,
        review_id: i64) -> AppResult<()>;                                  // spec 19

    // --- thread mutations ---------------------------------------------------
    /// `comment_id` = REST databaseId of the thread's root comment.
    fn reply_to_thread(&self, owner: &str, name: &str, number: i64,
        comment_id: i64, body: &str) -> AppResult<i64>;                    // spec 18
    /// `thread_id` = the opaque thread id from `pr_review_threads` (GitHub:
    /// GraphQL node id). Covers both resolve and unresolve.
    fn set_thread_resolved(&self, thread_id: &str, resolved: bool) -> AppResult<bool>; // spec 18
}

/// GitHub, via the `gh` CLI. Stateless: auth and host config live in `gh` itself.
pub struct GithubProvider;

impl ReviewProvider for GithubProvider {
    fn name(&self) -> &'static str { "github" }
    fn auth_status(&self) -> bool { gh::auth_status() }
    fn list_prs(&self, ctx: &GhRepo) -> AppResult<Vec<PrSummary>> { gh::list_prs(ctx) }
    fn pr_view(&self, ctx: &GhRepo, number: i64) -> AppResult<PrInfo> { gh::pr_view(ctx, number) }
    // … every remaining method is the same one-line delegation to the gh.rs
    // function of the same name and argument order. No logic, no logging, no
    // error mapping — pure pass-through.
}

// Compile-time object-safety assertion: if a future edit breaks dyn-compat
// (generic method, missing &self), this line fails to build.
const _: Option<&dyn ReviewProvider> = None;

/// The forge provider. Takes no argument today: nothing in the schema encodes a
/// host yet, so every repo is GitHub. When a second forge lands, give this the
/// real discriminator (e.g. a `repository.host` column or the remote URL) and
/// let the compiler enumerate the call sites.
pub fn provider_for() -> &'static dyn ReviewProvider {
    &GithubProvider
}
```

Also write the **extension recipe** as a comment block at the bottom of `provider.rs` (content in
§"Adding a GitLabProvider" below) and a `mod tests` with the two tests from the test matrix.

### 2. `src-tauri/src/lib.rs`

One line: `mod provider;` in the module list (`lib.rs:1-10`, alphabetical — between `mod
path_env;` and `mod tools;`). No `generate_handler!` change (no new commands).

### 3. Rewire `src-tauri/src/commands/gh.rs`

Add `use crate::provider::provider_for;`. Replace each `gh::<fn>(…)` call with
`provider_for().<fn>(…)`; keep everything else byte-identical (doc comments, `async`, arg
names — Tauri arg mapping must not move):

| Site (pre-wave-5 anchor) | Before | After |
|---|---|---|
| `gh_auth_status` (`:10-12`) | `gh::auth_status()` | `provider_for().auth_status()` |
| `list_prs` (`:14-17`) | `gh::list_prs(&GhRepo::Local(…))` | `provider_for().list_prs(&GhRepo::Local(…))` |
| `pr_meta` (`:23-26`) | `gh::pr_meta(&owner, &name, number)` | `provider_for().pr_meta(&owner, &name, number)` |
| `pr_review_threads` (`:30-37`) | `gh::pr_review_threads(…)` | `provider_for().pr_review_threads(…)` |
| `check_environment` (`:48-55`) | `gh_authed: gh::auth_status()` | `gh_authed: provider_for().auth_status()` |
| spec 18 `reply_to_thread` | `gh::reply_to_thread(…)` | `provider_for().reply_to_thread(…)` |
| spec 18 `set_pr_thread_resolved` | `gh::set_thread_resolved(…)` | `provider_for().set_thread_resolved(…)` |

Return-type references like `AppResult<Vec<gh::PrSummary>>` / `AppResult<gh::PrMeta>` stay as-is
(types live in `gh.rs`; only *function calls* are rewired).

### 4. Rewire `src-tauri/src/commands/review.rs`

Add `use crate::provider::provider_for;`. Same mechanical substitution at every `gh::` *call*
(type references `gh::PrInfo` `:161`/`:1071`, `gh::ComparedFile` `:325`, and spec 17's
`&gh::ReviewComment` in `AnchorKey::remote` stay untouched):

| Function (pre-wave-5 anchor) | Calls rewired |
|---|---|
| `refresh_target_shas` (`:214-261`) | `pr_view` (`:226`); spec 10 adds `merge_base_sha` here |
| `reanchor_review_comments` (`:292-403`) | `compare` (`:341`) |
| `create_review_for_pr` (`:446-464`) | `pr_view` (`:460`); spec 10 adds `merge_base_sha` |
| `review_diff` (`:469-488`) | `pr_diff` (`:479`) |
| `file_source` (`:495-538`) | `file_at_ref` (`:533`); spec 10 adds `merge_base_sha` backfill |
| `publish_review` (`:835-885`) | `post_review` (`:876`) |
| spec 17 `capture_github_comment_ids` | `review_comments` |
| spec 19 `publish_review_pending` | `post_review` (note: spec 19's `map_pending_conflict` wraps this call — keep the wrapping) |
| spec 19 `submit_pending_review` / `discard_pending_review` | `submit_pending_review` / `delete_pending_review` |

`gh_ctx_for_repo` (`:66-86`) keeps building `GhRepo` exactly as today (including the `github:`
sentinel parsing) — the context type is part of the v1 trait contract.

Resulting invariant (this is the spec-specific gate): **after this spec, `gh::` appears in
`src-tauri/src/commands/` only in type position** (`gh::PrInfo`, `gh::PrSummary`, `gh::PrMeta`,
`gh::PrThread`, `gh::ComparedFile`, `gh::ReviewComment`, `gh::GhRepo`/`GhRepo`) — never as a
function call.

### 5. Adding a `GitLabProvider` later (recipe — documentation only, goes in `provider.rs`)

1. **Host discrimination**: add a `repository.host TEXT NOT NULL DEFAULT 'github'` column (new
   append-only migration at the then-next number — 0007/0008/0009 are reserved by specs
   12/16/19), populated at `add_repository` time from the remote URL. Grow
   `provider_for(host: &str)`; the compiler enumerates every call site, each of which must then
   fetch/thread the host (the owner/name-addressed commands `pr_meta`/`pr_review_threads`/thread
   mutations will need the host passed from the frontend alongside owner/name).
2. **CLI wrapper module**: new `src-tauri/src/gitlab.rs` mirroring `gh.rs`'s shape around the
   `glab` CLI. Register the binary in `tools.rs` next to `gh_bin()` so GUI-launch PATH recovery
   (`path_env::ensure_login_path()`, see CLAUDE.md) covers it, and surface it in
   `check_environment`'s `ToolEnv`.
3. **Implement the trait**: `pub struct GitlabProvider;` in `provider.rs`, delegating to
   `gitlab.rs`. Map GitLab vocabulary (MRs, discussions, approve rules) into the trait's
   contracts.
4. **Pay down the v1 leaks**, in whatever order the implementation forces — this is the real
   work, catalogued here so nobody thinks the trait alone suffices:
   - `GhRepo` → rename to a neutral `RepoCtx` (shape is already provider-neutral:
     local-clone path vs remote owner/name);
   - `post_review(payload_json)` → a structured, provider-neutral publish payload
     (today `build_publish_payload`, `review.rs:709-750`, emits the GitHub REST shape;
     spec 17's matcher also assumes it);
   - submit-event vocabulary `APPROVE`/`REQUEST_CHANGES`/`COMMENT` (`gh_event`, spec 19);
   - DTO field semantics: `PrThread.id` is "the opaque id `pr_review_threads` returned"
     (GitHub: GraphQL node id), `reply_to_thread`'s `comment_id` is "the root comment's
     databaseId", `ReviewComment` mirrors GitHub REST fields, `side ∈ {LEFT, RIGHT}`;
   - the `github:owner/name` repo-path sentinel (`review.rs:55`, `:72`), `target.kind =
     'github_pr'`, and the `github_review_id`/`github_comment_id` column names (keep the
     columns, document them as "forge review/comment id");
   - `inbox.rs` (raw GitHub GraphQL search) and `check_environment` remain GitHub-only
     features until separately generalized.

## Tasks

1. **Reconciliation pass (mandatory, before any code):** run
   `grep -n "gh::" src-tauri/src/commands/*.rs` on the merged tree and diff the hits against the
   trait listing above. Adopt the *merged* signatures of specs 10/17/18/19 verbatim (names, arg
   order, return types); drop trait methods for anything that didn't land; add methods for any
   `gh.rs` function `commands/` calls that this spec missed. Record deltas in the commit message.
2. Add `src-tauri/src/provider.rs` (trait + `GithubProvider` + `provider_for` + object-safety
   assertion + recipe comment + tests) and `mod provider;` in `lib.rs`. Builds green standalone
   (`cargo clippy`/`cargo test` — the new code is exercised by the two unit tests).
3. Rewire `src-tauri/src/commands/gh.rs` per §3. Full gate suite green.
4. Rewire `src-tauri/src/commands/review.rs` per §4. Full gate suite green; verify the
   "type-position only" grep invariant.
5. `ROADMAP.md`: drop the §3 "Provider abstraction" bullet (repo convention:
   `docs(roadmap): drop items shipped …`).

Each step is an independently committable unit; commit directly to `main` (repo convention).

## Test matrix

No behavior changes, so the existing Rust suite (anchor/remap, payload, target, matcher, migration
and fixture-parse tests) is the regression net and must pass **unmodified** — if any existing test
needs editing, the rewiring was not mechanical; stop and re-check.

New, in `provider.rs::tests` (no network, no DB):

| Test | Asserts |
|---|---|
| `provider_for_is_github` | `provider_for().name() == "github"` — the factory returns a usable trait object and dispatch works |
| `github_provider_is_object_safe` | the `const _: Option<&dyn ReviewProvider> = None;` item compiles (compile-time; keep it at module scope, not inside `tests`) plus a runtime sanity pass of `&GithubProvider as &dyn ReviewProvider` to a helper fn calling `name()` |

vitest: none — no frontend change of any kind.

## Gates

Standard suite (all must pass):

```bash
pnpm exec tsc --noEmit
pnpm build
pnpm test
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test   --manifest-path src-tauri/Cargo.toml
```

Spec-specific:

- `grep -rn "gh::" src-tauri/src/commands/ | grep -vE "gh::(PrInfo|PrSummary|PrMeta|PrThread|ComparedFile|ReviewComment|GhRepo)"`
  → no output (only type-position references remain).
- `git diff --stat src-tauri/src/gh.rs src-tauri/src/git.rs src-tauri/src/inbox.rs src/` → empty
  (`gh.rs` bodies, local git, inbox, and the entire frontend are untouched).
- No new migration files in the diff.

## Manual verify

Zero behavior change means this is a smoke pass over one representative call per trait method
group (`pnpm tauri dev`, `gh` authenticated):

1. Settings/diagnostics shows `gh` authenticated (`auth_status`).
2. Open a repo's PR list — PRs load (`list_prs`).
3. Open a PR review — diff, header metadata and existing GitHub threads render
   (`pr_view`, `pr_diff`, `pr_meta`, `pr_review_threads`).
4. Expand collapsed context on a modified file in the PR (`file_at_ref`, and `merge_base_sha`
   via spec 10's backfill); click **Refresh** (`pr_view` again).
5. On a scratch PR (reuse spec 18's fixture recipe): reply to a thread and resolve/unresolve it
   (`reply_to_thread`, `set_thread_resolved`).
6. Publish a small review to the scratch PR — succeeds, and the terminal shows spec 17's
   `[publish.capture_ids] stored N github comment ids …` line (`post_review`,
   `review_comments`). If spec 19 landed: stage a pending review, then submit or discard it
   (`submit_pending_review` / `delete_pending_review`).
7. Sanity: a local virtual-PR review still diffs/comments/exports exactly as before (the local
   path never touches the provider).

## Out of scope

- **Any second provider implementation** (GitLab/Bitbucket/Gitea), the `glab` wrapper module, a
  `repository.host` column/migration, or frontend host awareness — the recipe documents the path;
  this spec only builds the seam.
- **Type neutralization**: renaming `GhRepo`, structuring `post_review`'s payload, neutral event
  vocabulary, renaming `github_*` DB columns or the `github:` path sentinel, `kind='github_pr'`.
- **`inbox.rs`** (raw GraphQL search + `pr_files`) and **`gh::graphql`** — stay concrete GitHub.
- **`git.rs`** — local git is not a forge; stays concrete everywhere.
- **`export.rs`** and all frontend code — untouched.
- Behavior changes of any kind: no retries, no logging additions inside delegations, no error-type
  changes, no new commands, no `generate_handler!` edits.
