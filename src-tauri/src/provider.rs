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
    /// Stable short tag for logs/tests ("github"). Only the tests consume it
    /// today; kept on the trait so a second provider self-identifies in logs.
    #[allow(dead_code)]
    fn name(&self) -> &'static str;

    // --- auth ---------------------------------------------------------------
    fn auth_status(&self) -> bool;

    // --- PR reads -----------------------------------------------------------
    fn list_prs(&self, ctx: &GhRepo) -> AppResult<Vec<PrSummary>>;
    fn pr_view(&self, ctx: &GhRepo, number: i64) -> AppResult<PrInfo>;
    fn pr_diff(&self, ctx: &GhRepo, number: i64) -> AppResult<String>;
    fn pr_meta(&self, owner: &str, name: &str, number: i64) -> AppResult<PrMeta>;
    fn pr_review_threads(
        &self,
        owner: &str,
        name: &str,
        number: i64,
    ) -> AppResult<Vec<PrThread>>;
    fn compare(
        &self,
        owner: &str,
        name: &str,
        base: &str,
        head: &str,
    ) -> AppResult<Vec<ComparedFile>>;
    fn merge_base_sha(&self, owner: &str, name: &str, base: &str, head: &str) -> AppResult<String>; // spec 10
    fn file_at_ref(
        &self,
        owner: &str,
        name: &str,
        file_path: &str,
        git_ref: &str,
    ) -> AppResult<String>;

    // --- review writes ------------------------------------------------------
    /// `payload_json` is the GitHub REST reviews payload built by
    /// `build_publish_payload` — a v1 leak, kept verbatim (zero behavior change).
    fn post_review(
        &self,
        owner: &str,
        name: &str,
        number: i64,
        payload_json: &str,
    ) -> AppResult<i64>;
    fn review_comments(
        &self,
        owner: &str,
        name: &str,
        number: i64,
        review_id: i64,
    ) -> AppResult<Vec<ReviewComment>>; // spec 17
    /// `event` ∈ APPROVE | REQUEST_CHANGES | COMMENT (GitHub's vocabulary — v1 leak).
    fn submit_pending_review(
        &self,
        owner: &str,
        name: &str,
        number: i64,
        review_id: i64,
        event: &str,
    ) -> AppResult<()>; // spec 19
    fn delete_pending_review(
        &self,
        owner: &str,
        name: &str,
        number: i64,
        review_id: i64,
    ) -> AppResult<()>; // spec 19

    // --- thread mutations ---------------------------------------------------
    /// `comment_id` = REST databaseId of the thread's root comment.
    fn reply_to_thread(
        &self,
        owner: &str,
        name: &str,
        number: i64,
        comment_id: i64,
        body: &str,
    ) -> AppResult<i64>; // spec 18
    /// `thread_id` = the opaque thread id from `pr_review_threads` (GitHub:
    /// GraphQL node id). Covers both resolve and unresolve.
    fn set_thread_resolved(&self, thread_id: &str, resolved: bool) -> AppResult<bool>; // spec 18
}

/// GitHub, via the `gh` CLI. Stateless: auth and host config live in `gh` itself.
pub struct GithubProvider;

impl ReviewProvider for GithubProvider {
    fn name(&self) -> &'static str {
        "github"
    }

    fn auth_status(&self) -> bool {
        gh::auth_status()
    }

    fn list_prs(&self, ctx: &GhRepo) -> AppResult<Vec<PrSummary>> {
        gh::list_prs(ctx)
    }

    fn pr_view(&self, ctx: &GhRepo, number: i64) -> AppResult<PrInfo> {
        gh::pr_view(ctx, number)
    }

    fn pr_diff(&self, ctx: &GhRepo, number: i64) -> AppResult<String> {
        gh::pr_diff(ctx, number)
    }

    fn pr_meta(&self, owner: &str, name: &str, number: i64) -> AppResult<PrMeta> {
        gh::pr_meta(owner, name, number)
    }

    fn pr_review_threads(
        &self,
        owner: &str,
        name: &str,
        number: i64,
    ) -> AppResult<Vec<PrThread>> {
        gh::pr_review_threads(owner, name, number)
    }

    fn compare(
        &self,
        owner: &str,
        name: &str,
        base: &str,
        head: &str,
    ) -> AppResult<Vec<ComparedFile>> {
        gh::compare(owner, name, base, head)
    }

    fn merge_base_sha(&self, owner: &str, name: &str, base: &str, head: &str) -> AppResult<String> {
        gh::merge_base_sha(owner, name, base, head)
    }

    fn file_at_ref(
        &self,
        owner: &str,
        name: &str,
        file_path: &str,
        git_ref: &str,
    ) -> AppResult<String> {
        gh::file_at_ref(owner, name, file_path, git_ref)
    }

    fn post_review(
        &self,
        owner: &str,
        name: &str,
        number: i64,
        payload_json: &str,
    ) -> AppResult<i64> {
        gh::post_review(owner, name, number, payload_json)
    }

    fn review_comments(
        &self,
        owner: &str,
        name: &str,
        number: i64,
        review_id: i64,
    ) -> AppResult<Vec<ReviewComment>> {
        gh::review_comments(owner, name, number, review_id)
    }

    fn submit_pending_review(
        &self,
        owner: &str,
        name: &str,
        number: i64,
        review_id: i64,
        event: &str,
    ) -> AppResult<()> {
        gh::submit_pending_review(owner, name, number, review_id, event)
    }

    fn delete_pending_review(
        &self,
        owner: &str,
        name: &str,
        number: i64,
        review_id: i64,
    ) -> AppResult<()> {
        gh::delete_pending_review(owner, name, number, review_id)
    }

    fn reply_to_thread(
        &self,
        owner: &str,
        name: &str,
        number: i64,
        comment_id: i64,
        body: &str,
    ) -> AppResult<i64> {
        gh::reply_to_thread(owner, name, number, comment_id, body)
    }

    fn set_thread_resolved(&self, thread_id: &str, resolved: bool) -> AppResult<bool> {
        gh::set_thread_resolved(thread_id, resolved)
    }
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

// ---------------------------------------------------------------------------
// Adding a `GitLabProvider` (or any second forge) later — recipe, not yet built
// ---------------------------------------------------------------------------
//
// 1. Host discrimination: add a `repository.host TEXT NOT NULL DEFAULT 'github'`
//    column (new append-only migration at the then-next number — 0007/0008/0009
//    are reserved by specs 12/16/19), populated at `add_repository` time from
//    the remote URL. Grow `provider_for(host: &str)`; the compiler enumerates
//    every call site, each of which must then fetch/thread the host (the
//    owner/name-addressed commands `pr_meta`/`pr_review_threads`/thread
//    mutations will need the host passed from the frontend alongside owner/name).
//
// 2. CLI wrapper module: new `src-tauri/src/gitlab.rs` mirroring `gh.rs`'s shape
//    around the `glab` CLI. Register the binary in `tools.rs` next to `gh_bin()`
//    so GUI-launch PATH recovery (`path_env::ensure_login_path()`, see CLAUDE.md)
//    covers it, and surface it in `check_environment`'s `ToolEnv`.
//
// 3. Implement the trait: `pub struct GitlabProvider;` in `provider.rs`,
//    delegating to `gitlab.rs`. Map GitLab vocabulary (MRs, discussions, approve
//    rules) into the trait's contracts.
//
// 4. Pay down the v1 leaks, in whatever order the implementation forces — this
//    is the real work, catalogued here so nobody thinks the trait alone suffices:
//    - `GhRepo` → rename to a neutral `RepoCtx` (shape is already
//      provider-neutral: local-clone path vs remote owner/name);
//    - `post_review(payload_json)` → a structured, provider-neutral publish
//      payload (today `build_publish_payload`, `review.rs`, emits the GitHub
//      REST shape; spec 17's matcher also assumes it);
//    - submit-event vocabulary `APPROVE`/`REQUEST_CHANGES`/`COMMENT` (`gh_event`,
//      spec 19);
//    - DTO field semantics: `PrThread.id` is "the opaque id `pr_review_threads`
//      returned" (GitHub: GraphQL node id), `reply_to_thread`'s `comment_id` is
//      "the root comment's databaseId", `ReviewComment` mirrors GitHub REST
//      fields, `side ∈ {LEFT, RIGHT}`;
//    - the `github:owner/name` repo-path sentinel, `target.kind = 'github_pr'`,
//      and the `github_review_id`/`github_comment_id` column names (keep the
//      columns, document them as "forge review/comment id");
//    - `inbox.rs` (raw GitHub GraphQL search) and `check_environment` remain
//      GitHub-only features until separately generalized.

#[cfg(test)]
mod tests {
    use super::*;

    fn provider_name(p: &dyn ReviewProvider) -> &'static str {
        p.name()
    }

    #[test]
    fn provider_for_is_github() {
        assert_eq!(provider_for().name(), "github");
    }

    #[test]
    fn github_provider_is_object_safe() {
        // Runtime sanity that `&GithubProvider` coerces to a trait object and
        // dispatch works; the module-scope `const _` above proves object-safety
        // at compile time.
        let obj: &dyn ReviewProvider = &GithubProvider;
        assert_eq!(provider_name(obj), "github");
    }
}
