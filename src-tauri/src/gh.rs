use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// How a `gh` invocation resolves its repository. `Local` runs `gh` with the
/// clone as the working directory (so `gh` reads the repo from `origin`);
/// `Remote` runs clone-less by passing `--repo owner/name` and using a neutral
/// working directory. Absolute `gh api repos/{owner}/{name}/...` endpoints ignore
/// the cwd, so they work under either variant.
pub enum GhRepo {
    Local(PathBuf),
    Remote { owner: String, name: String },
}

impl GhRepo {
    /// Working directory for the `gh` process. A real clone for `Local`; a
    /// neutral temp dir for `Remote` (never read when `--repo`/an absolute
    /// endpoint is given).
    fn cwd(&self) -> PathBuf {
        match self {
            GhRepo::Local(p) => p.clone(),
            GhRepo::Remote { .. } => std::env::temp_dir(),
        }
    }

    /// Args appended to `gh pr ...` subcommands so they target the right repo
    /// when there is no local clone. Centralized here so no `pr` call can forget
    /// it.
    fn pr_args_suffix(&self) -> Vec<String> {
        match self {
            GhRepo::Local(_) => Vec::new(),
            GhRepo::Remote { owner, name } => {
                vec!["--repo".into(), format!("{owner}/{name}")]
            }
        }
    }
}

/// Run a `gh` command in the given repo context. Returns stdout on success.
pub fn run_gh(ctx: &GhRepo, args: &[&str]) -> AppResult<String> {
    let output = Command::new(crate::tools::gh_bin())
        .current_dir(ctx.cwd())
        .args(args)
        .output()
        .map_err(|e| AppError::Gh(format!("failed to spawn gh: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Gh(stderr.trim().to_string()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Run a `gh` command feeding `input` to its stdin (used for `gh api --input -`).
pub fn run_gh_stdin(ctx: &GhRepo, args: &[&str], input: &str) -> AppResult<String> {
    let mut child = Command::new(crate::tools::gh_bin())
        .current_dir(ctx.cwd())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| AppError::Gh(format!("failed to spawn gh: {e}")))?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppError::Gh("failed to open gh stdin".into()))?;
        stdin.write_all(input.as_bytes())?;
    } // stdin dropped here -> EOF

    let output = child
        .wait_with_output()
        .map_err(|e| AppError::Gh(format!("gh failed: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Gh(stderr.trim().to_string()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Envelope returned by GitHub's GraphQL API. Field-level failures arrive as a
/// 200 response carrying both `data` and `errors`, so we must inspect the body
/// rather than trust `gh`'s exit code.
#[derive(Deserialize)]
struct GraphQlEnvelope<T> {
    data: Option<T>,
    errors: Option<serde_json::Value>,
}

/// Run a GraphQL query through `gh api graphql`. The `{query, variables}` body is
/// piped via stdin (`--input -`) so the multi-line query needs no shell quoting
/// and array variables pass as real JSON. Tolerates partial responses: if `data`
/// is present alongside `errors`, the data is returned and the errors are logged.
pub fn graphql<T: DeserializeOwned>(query: &str, variables: serde_json::Value) -> AppResult<T> {
    let body = serde_json::json!({ "query": query, "variables": variables }).to_string();
    // Absolute endpoint; cwd is irrelevant, so a throwaway Remote ctx is fine.
    let ctx = GhRepo::Remote {
        owner: String::new(),
        name: String::new(),
    };
    let out = run_gh_stdin(&ctx, &["api", "graphql", "--input", "-"], &body)?;
    let env: GraphQlEnvelope<T> = serde_json::from_str(&out)
        .map_err(|e| AppError::Gh(format!("failed to parse graphql response: {e}")))?;
    match (env.data, env.errors) {
        (Some(data), errors) => {
            if let Some(errors) = errors {
                eprintln!("[gh.graphql] partial errors: {errors}");
            }
            Ok(data)
        }
        (None, Some(errors)) => Err(AppError::Gh(format!("graphql errors: {errors}"))),
        (None, None) => Err(AppError::Gh("graphql returned no data".into())),
    }
}

/// POST a review (body + verdict + inline comments) to a PR. Returns the new
/// GitHub review id. Uses an absolute endpoint, so it works without a local clone.
pub fn post_review(owner: &str, name: &str, number: i64, payload_json: &str) -> AppResult<i64> {
    let endpoint = format!("repos/{owner}/{name}/pulls/{number}/reviews");
    let ctx = GhRepo::Remote {
        owner: owner.to_string(),
        name: name.to_string(),
    };
    let out = run_gh_stdin(
        &ctx,
        &["api", &endpoint, "--method", "POST", "--input", "-"],
        payload_json,
    )?;
    let value: serde_json::Value = serde_json::from_str(&out)
        .map_err(|e| AppError::Gh(format!("failed to parse review response: {e}")))?;
    Ok(value.get("id").and_then(|v| v.as_i64()).unwrap_or_default())
}

/// One inline comment of a submitted review, from the REST
/// `pulls/{n}/reviews/{review_id}/comments` API. Anchor fields are Option-typed
/// defensively: the matcher skips items missing side/line rather than erroring.
#[derive(Debug, Deserialize)]
pub struct ReviewComment {
    pub id: i64,
    pub path: String,
    #[serde(default)]
    pub side: Option<String>, // "LEFT" | "RIGHT"
    #[serde(default)]
    pub line: Option<i64>,
    #[serde(default)]
    pub start_line: Option<i64>, // null for single-line comments
    #[serde(default)]
    pub body: String,
}

/// All inline comments belonging to one review. Explicit per_page/page loop;
/// stops when a page returns fewer than 100 items (mirrors `pr_review_threads`'
/// house style rather than relying on `gh api --paginate`). Clone-less.
pub fn review_comments(
    owner: &str,
    name: &str,
    number: i64,
    review_id: i64,
) -> AppResult<Vec<ReviewComment>> {
    let ctx = GhRepo::Remote {
        owner: owner.to_string(),
        name: name.to_string(),
    };
    let mut all = Vec::new();
    let mut page = 1u32;
    loop {
        let endpoint = format!(
            "repos/{owner}/{name}/pulls/{number}/reviews/{review_id}/comments?per_page=100&page={page}"
        );
        let out = run_gh(&ctx, &["api", &endpoint])?;
        let batch: Vec<ReviewComment> = serde_json::from_str(&out)
            .map_err(|e| AppError::Gh(format!("failed to parse review comments: {e}")))?;
        let n = batch.len();
        all.extend(batch);
        if n < 100 {
            break;
        }
        page += 1;
    }
    Ok(all)
}

/// Full contents of `file_path` at `git_ref` via the GitHub contents API,
/// requesting the raw media type so the body is the file itself (not base64 JSON).
/// Used as a fallback when a PR's commit isn't present in the local clone, and as
/// the primary source for remote-only (clone-less) PR targets.
pub fn file_at_ref(owner: &str, name: &str, file_path: &str, git_ref: &str) -> AppResult<String> {
    let endpoint = format!("repos/{owner}/{name}/contents/{file_path}?ref={git_ref}");
    let ctx = GhRepo::Remote {
        owner: owner.to_string(),
        name: name.to_string(),
    };
    run_gh(
        &ctx,
        &["api", &endpoint, "-H", "Accept: application/vnd.github.raw"],
    )
}

pub fn auth_status() -> bool {
    Command::new(crate::tools::gh_bin())
        .args(["auth", "status"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PrAuthor {
    pub login: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PrSummary {
    pub number: i64,
    pub title: String,
    #[serde(default)]
    pub author: Option<PrAuthor>,
    #[serde(rename = "headRefName")]
    pub head_ref_name: String,
    #[serde(rename = "baseRefName")]
    pub base_ref_name: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    pub url: String,
}

pub fn list_prs(ctx: &GhRepo) -> AppResult<Vec<PrSummary>> {
    let mut args = vec![
        "pr",
        "list",
        "--state",
        "open",
        "--limit",
        "100",
        "--json",
        "number,title,author,headRefName,baseRefName,createdAt,url",
    ];
    let suffix = ctx.pr_args_suffix();
    args.extend(suffix.iter().map(String::as_str));
    let out = run_gh(ctx, &args)?;
    serde_json::from_str(&out).map_err(|e| AppError::Gh(format!("failed to parse pr list: {e}")))
}

/// One entry of a PR's changed-file list, from the REST `pulls/{n}/files` API.
#[derive(Debug, Deserialize, Serialize)]
pub struct ChangedFile {
    pub filename: String,
    pub additions: i64,
    pub deletions: i64,
    pub changes: i64,
}

/// The changed files of a PR (first page, up to 100) via the REST API. Used to
/// enrich inbox PR rows with a top-changed-files preview. Works clone-less.
pub fn pr_files(owner: &str, name: &str, number: i64) -> AppResult<Vec<ChangedFile>> {
    let endpoint = format!("repos/{owner}/{name}/pulls/{number}/files?per_page=100");
    let ctx = GhRepo::Remote {
        owner: owner.to_string(),
        name: name.to_string(),
    };
    let out = run_gh(&ctx, &["api", &endpoint])?;
    serde_json::from_str(&out).map_err(|e| AppError::Gh(format!("failed to parse pr files: {e}")))
}

/// One changed file from the REST `compare/{base}...{head}` API, carrying its
/// per-file unified-diff `patch` (absent for binary/large files).
#[derive(Debug, Deserialize)]
pub struct ComparedFile {
    pub filename: String,
    #[serde(default)]
    pub patch: Option<String>,
    // Part of the compare contract (Spec 01 §2); surfaced for callers/tests even
    // though the re-anchor helper keys only on filename + patch.
    #[allow(dead_code)]
    pub status: String,
}

/// The merge-base commit reference embedded in a REST compare response.
#[derive(Debug, Deserialize)]
struct CommitRef {
    sha: String,
}

#[derive(Debug, Deserialize)]
struct CompareRaw {
    #[serde(default)]
    merge_base_commit: Option<CommitRef>,
    files: Vec<ComparedFile>,
}

/// `gh api repos/{owner}/{name}/compare/{base}...{head}` → changed files with
/// per-file patches. Three-dot compare semantics (GitHub's own "files changed").
/// Clone-less; ignores `files` pagination (compare returns up to 300 per page).
pub fn compare(owner: &str, name: &str, base: &str, head: &str) -> AppResult<Vec<ComparedFile>> {
    let endpoint = format!("repos/{owner}/{name}/compare/{base}...{head}");
    let ctx = GhRepo::Remote {
        owner: owner.to_string(),
        name: name.to_string(),
    };
    let out = run_gh(&ctx, &["api", &endpoint])?;
    let raw: CompareRaw =
        serde_json::from_str(&out).map_err(|e| AppError::Gh(format!("failed to parse compare: {e}")))?;
    Ok(raw.files)
}

/// Merge-base of `base` and `head` via the REST compare API (clone-less).
/// `?per_page=1` caps the commits payload; only `merge_base_commit.sha` is read.
/// This is the LEFT side of GitHub's three-dot PR diff — NOT the base-branch tip.
pub fn merge_base_sha(owner: &str, name: &str, base: &str, head: &str) -> AppResult<String> {
    let endpoint = format!("repos/{owner}/{name}/compare/{base}...{head}?per_page=1");
    let ctx = GhRepo::Remote {
        owner: owner.to_string(),
        name: name.to_string(),
    };
    let out = run_gh(&ctx, &["api", &endpoint])?;
    let raw: CompareRaw =
        serde_json::from_str(&out).map_err(|e| AppError::Gh(format!("failed to parse compare: {e}")))?;
    raw.merge_base_commit
        .map(|c| c.sha)
        .ok_or_else(|| AppError::Gh("compare response missing merge_base_commit".into()))
}

pub fn pr_diff(ctx: &GhRepo, number: i64) -> AppResult<String> {
    let num = number.to_string();
    let mut args = vec!["pr", "diff", num.as_str()];
    let suffix = ctx.pr_args_suffix();
    args.extend(suffix.iter().map(String::as_str));
    run_gh(ctx, &args)
}

#[derive(Debug, Deserialize)]
struct PrViewRaw {
    title: String,
    #[serde(rename = "baseRefName")]
    base_ref_name: String,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "headRefOid")]
    head_ref_oid: String,
}

#[derive(Debug)]
pub struct PrInfo {
    pub title: String,
    pub base_ref: String,
    pub head_ref: String,
    pub head_sha: String,
}

pub fn pr_view(ctx: &GhRepo, number: i64) -> AppResult<PrInfo> {
    let num = number.to_string();
    let mut args = vec![
        "pr",
        "view",
        num.as_str(),
        "--json",
        "title,baseRefName,headRefName,headRefOid",
    ];
    let suffix = ctx.pr_args_suffix();
    args.extend(suffix.iter().map(String::as_str));
    let out = run_gh(ctx, &args)?;
    let raw: PrViewRaw =
        serde_json::from_str(&out).map_err(|e| AppError::Gh(format!("failed to parse pr view: {e}")))?;
    Ok(PrInfo {
        title: raw.title,
        base_ref: raw.base_ref_name,
        head_ref: raw.head_ref_name,
        head_sha: raw.head_ref_oid,
    })
}

// ---------------------------------------------------------------------------
// PR metadata (clone-less, for the review header)
// ---------------------------------------------------------------------------

const PR_META_QUERY: &str = r#"
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
      commits(last: 1) {
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
"#;

/// PR metadata returned to the frontend (review header). Read-only; never persisted.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrMeta {
    pub number: i64,
    pub title: String,
    pub url: String,
    pub body: String,
    pub state: String,
    pub is_draft: bool,
    pub mergeable: Option<String>,
    pub review_decision: Option<String>,
    pub additions: i64,
    pub deletions: i64,
    pub changed_files: i64,
    pub author: Option<PrActor>,
    pub labels: Vec<PrLabel>,
    pub reviews: Vec<PrReviewer>,
    pub ci_state: Option<String>,
    pub checks: Vec<PrCheck>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PrActor {
    pub login: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PrLabel {
    pub name: String,
    pub color: String,
}

#[derive(Debug, Serialize)]
pub struct PrReviewer {
    pub author: Option<PrActor>,
    pub state: String,
}

#[derive(Debug, Serialize)]
pub struct PrCheck {
    pub name: String,
    pub state: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PrMetaData {
    repository: Option<PrMetaRepo>,
}

#[derive(Debug, Deserialize)]
struct PrMetaRepo {
    #[serde(rename = "pullRequest")]
    pull_request: Option<PrMetaPr>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrMetaPr {
    number: i64,
    title: String,
    url: String,
    #[serde(default)]
    body: Option<String>,
    state: String,
    #[serde(default)]
    is_draft: bool,
    #[serde(default)]
    mergeable: Option<String>,
    #[serde(default)]
    review_decision: Option<String>,
    additions: i64,
    deletions: i64,
    changed_files: i64,
    #[serde(default)]
    author: Option<PrActor>,
    #[serde(default)]
    labels: LabelConn,
    #[serde(default)]
    latest_reviews: ReviewConn,
    #[serde(default)]
    commits: CommitMetaConn,
}

#[derive(Debug, Default, Deserialize)]
struct LabelConn {
    #[serde(default)]
    nodes: Vec<LabelNode>,
}

#[derive(Debug, Deserialize)]
struct LabelNode {
    name: String,
    color: String,
}

#[derive(Debug, Default, Deserialize)]
struct ReviewConn {
    #[serde(default)]
    nodes: Vec<ReviewNode>,
}

#[derive(Debug, Deserialize)]
struct ReviewNode {
    #[serde(default)]
    author: Option<PrActor>,
    state: String,
}

#[derive(Debug, Default, Deserialize)]
struct CommitMetaConn {
    #[serde(default)]
    nodes: Vec<CommitMetaNode>,
}

#[derive(Debug, Deserialize)]
struct CommitMetaNode {
    commit: CommitMetaInner,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommitMetaInner {
    #[serde(default)]
    status_check_rollup: Option<RollupNode>,
}

#[derive(Debug, Deserialize)]
struct RollupNode {
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    contexts: ContextConn,
}

#[derive(Debug, Default, Deserialize)]
struct ContextConn {
    #[serde(default)]
    nodes: Vec<ContextNode>,
}

/// A union node from `statusCheckRollup.contexts`: either a `CheckRun` (GitHub
/// Actions / app checks) or a legacy `StatusContext` (commit statuses).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextNode {
    #[serde(rename = "__typename")]
    typename: String,
    // CheckRun
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    details_url: Option<String>,
    // StatusContext
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    target_url: Option<String>,
}

fn map_pr_meta(pr: PrMetaPr) -> PrMeta {
    let labels = pr
        .labels
        .nodes
        .into_iter()
        .map(|l| PrLabel {
            name: l.name,
            color: l.color,
        })
        .collect();
    let reviews = pr
        .latest_reviews
        .nodes
        .into_iter()
        .map(|r| PrReviewer {
            author: r.author,
            state: r.state,
        })
        .collect();
    let rollup = pr
        .commits
        .nodes
        .into_iter()
        .next()
        .and_then(|c| c.commit.status_check_rollup);
    let ci_state = rollup.as_ref().and_then(|r| r.state.clone());
    let checks = rollup
        .map(|r| {
            r.contexts
                .nodes
                .into_iter()
                .map(|c| {
                    if c.typename == "CheckRun" {
                        PrCheck {
                            name: c.name.unwrap_or_default(),
                            // A finished CheckRun reports `conclusion`; an in-flight
                            // one only has `status`.
                            state: c.conclusion.or(c.status),
                            url: c.details_url,
                        }
                    } else {
                        PrCheck {
                            name: c.context.unwrap_or_default(),
                            state: c.state,
                            url: c.target_url,
                        }
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    PrMeta {
        number: pr.number,
        title: pr.title,
        url: pr.url,
        body: pr.body.unwrap_or_default(),
        state: pr.state,
        is_draft: pr.is_draft,
        mergeable: pr.mergeable,
        review_decision: pr.review_decision,
        additions: pr.additions,
        deletions: pr.deletions,
        changed_files: pr.changed_files,
        author: pr.author,
        labels,
        reviews,
        ci_state,
        checks,
    }
}

/// Fetch a PR's metadata for the review header. Clone-less (owner/name/number),
/// read-only, ephemeral — nothing is written to SQLite.
pub fn pr_meta(owner: &str, name: &str, number: i64) -> AppResult<PrMeta> {
    let vars = serde_json::json!({ "owner": owner, "name": name, "number": number });
    let data: PrMetaData = graphql(PR_META_QUERY, vars)?;
    let pr = data
        .repository
        .and_then(|r| r.pull_request)
        .ok_or_else(|| AppError::Gh(format!("PR {owner}/{name}#{number} not found")))?;
    Ok(map_pr_meta(pr))
}

// ---------------------------------------------------------------------------
// PR review threads (clone-less, read-only, ephemeral)
// ---------------------------------------------------------------------------

const PR_THREADS_QUERY: &str = r#"
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
"#;

/// An existing GitHub review thread on a PR's diff. Read-only; never persisted.
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
    pub diff_side: Option<String>,
    pub start_diff_side: Option<String>,
    pub subject_type: Option<String>,
    pub comments: Vec<PrThreadComment>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrThreadComment {
    pub id: String,
    pub database_id: Option<i64>,
    pub author: Option<PrActor>,
    pub body: String,
    pub created_at: String,
    pub url: String,
    pub diff_hunk: Option<String>,
    pub outdated: bool,
}

#[derive(Debug, Deserialize)]
struct PrThreadsData {
    repository: Option<PrThreadsRepo>,
}

#[derive(Debug, Deserialize)]
struct PrThreadsRepo {
    #[serde(rename = "pullRequest")]
    pull_request: Option<PrThreadsPr>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrThreadsPr {
    review_threads: ThreadConn,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadConn {
    page_info: PageInfo,
    nodes: Vec<ThreadNode>,
}

#[derive(Debug, Deserialize)]
struct PageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
    #[serde(rename = "endCursor")]
    end_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadNode {
    id: String,
    is_resolved: bool,
    is_outdated: bool,
    is_collapsed: bool,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    line: Option<i64>,
    #[serde(default)]
    start_line: Option<i64>,
    #[serde(default)]
    original_line: Option<i64>,
    #[serde(default)]
    diff_side: Option<String>,
    #[serde(default)]
    start_diff_side: Option<String>,
    #[serde(default)]
    subject_type: Option<String>,
    #[serde(default)]
    comments: ThreadCommentConn,
}

#[derive(Debug, Default, Deserialize)]
struct ThreadCommentConn {
    #[serde(default)]
    nodes: Vec<ThreadCommentNode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadCommentNode {
    id: String,
    #[serde(default)]
    database_id: Option<i64>,
    #[serde(default)]
    author: Option<PrActor>,
    #[serde(default)]
    body: Option<String>,
    created_at: String,
    url: String,
    #[serde(default)]
    diff_hunk: Option<String>,
    #[serde(default)]
    outdated: bool,
}

fn map_thread(t: ThreadNode) -> PrThread {
    // We take the first 100 comments per thread; threads longer than that are
    // vanishingly rare and replies beyond the cap are dropped rather than paginated.
    let comments = t
        .comments
        .nodes
        .into_iter()
        .map(|c| PrThreadComment {
            id: c.id,
            database_id: c.database_id,
            author: c.author,
            body: c.body.unwrap_or_default(),
            created_at: c.created_at,
            url: c.url,
            diff_hunk: c.diff_hunk,
            outdated: c.outdated,
        })
        .collect();
    PrThread {
        id: t.id,
        is_resolved: t.is_resolved,
        is_outdated: t.is_outdated,
        is_collapsed: t.is_collapsed,
        path: t.path,
        line: t.line,
        start_line: t.start_line,
        original_line: t.original_line,
        diff_side: t.diff_side,
        start_diff_side: t.start_diff_side,
        subject_type: t.subject_type,
        comments,
    }
}

/// Fetch a PR's existing review threads (inline diff comment threads). Clone-less
/// (owner/name/number), read-only, ephemeral — nothing is written to SQLite.
/// Paginates `reviewThreads` to completion so large PRs aren't silently truncated.
pub fn pr_review_threads(owner: &str, name: &str, number: i64) -> AppResult<Vec<PrThread>> {
    let mut threads = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let vars =
            serde_json::json!({ "owner": owner, "name": name, "number": number, "cursor": cursor });
        let data: PrThreadsData = graphql(PR_THREADS_QUERY, vars)?;
        let conn = data
            .repository
            .and_then(|r| r.pull_request)
            .map(|pr| pr.review_threads)
            .ok_or_else(|| AppError::Gh(format!("PR {owner}/{name}#{number} not found")))?;
        threads.extend(conn.nodes.into_iter().map(map_thread));
        if conn.page_info.has_next_page {
            match conn.page_info.end_cursor {
                Some(c) => cursor = Some(c),
                None => break,
            }
        } else {
            break;
        }
    }
    Ok(threads)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
      "repository": {
        "pullRequest": {
          "number": 42,
          "title": "Add widgets",
          "url": "https://github.com/acme/widget/pull/42",
          "body": "Summary line.\nDoes a thing.",
          "state": "OPEN",
          "isDraft": false,
          "mergeable": "MERGEABLE",
          "reviewDecision": null,
          "additions": 120,
          "deletions": 7,
          "changedFiles": 5,
          "author": { "login": "octocat", "avatarUrl": "https://example.com/a.png" },
          "labels": { "nodes": [
            { "name": "bug", "color": "d73a4a" },
            { "name": "wip", "color": "ededed" }
          ] },
          "latestReviews": { "nodes": [
            { "author": { "login": "rev1", "avatarUrl": "https://example.com/r1.png" }, "state": "APPROVED" },
            { "author": { "login": "rev2", "avatarUrl": null }, "state": "CHANGES_REQUESTED" }
          ] },
          "commits": { "nodes": [
            { "commit": { "statusCheckRollup": {
              "state": "FAILURE",
              "contexts": { "nodes": [
                { "__typename": "CheckRun", "name": "build", "conclusion": "SUCCESS", "status": "COMPLETED", "detailsUrl": "https://ci/build" },
                { "__typename": "CheckRun", "name": "test", "conclusion": null, "status": "IN_PROGRESS", "detailsUrl": "https://ci/test" },
                { "__typename": "StatusContext", "context": "legacy/lint", "state": "FAILURE", "targetUrl": "https://ci/lint" }
              ] }
            } } }
          ] }
        }
      }
    }"#;

    #[test]
    fn pr_meta_maps_fixture() {
        let data: PrMetaData = serde_json::from_str(FIXTURE).expect("fixture parses");
        let pr = data.repository.unwrap().pull_request.unwrap();
        let meta = map_pr_meta(pr);

        assert_eq!(meta.number, 42);
        assert_eq!(meta.state, "OPEN");
        assert!(!meta.is_draft);
        assert_eq!(meta.mergeable.as_deref(), Some("MERGEABLE"));
        assert_eq!(meta.review_decision, None);
        assert_eq!(meta.additions, 120);
        assert_eq!(meta.changed_files, 5);
        assert_eq!(meta.author.as_ref().and_then(|a| a.login.as_deref()), Some("octocat"));

        let labels: Vec<&str> = meta.labels.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(labels, vec!["bug", "wip"]);

        assert_eq!(meta.reviews.len(), 2);
        assert_eq!(meta.reviews[0].state, "APPROVED");

        assert_eq!(meta.ci_state.as_deref(), Some("FAILURE"));
        assert_eq!(meta.checks.len(), 3);
        // Finished CheckRun -> conclusion.
        assert_eq!(meta.checks[0].name, "build");
        assert_eq!(meta.checks[0].state.as_deref(), Some("SUCCESS"));
        // In-flight CheckRun -> falls back to status.
        assert_eq!(meta.checks[1].name, "test");
        assert_eq!(meta.checks[1].state.as_deref(), Some("IN_PROGRESS"));
        // Legacy StatusContext -> context/state/targetUrl.
        assert_eq!(meta.checks[2].name, "legacy/lint");
        assert_eq!(meta.checks[2].state.as_deref(), Some("FAILURE"));
        assert_eq!(meta.checks[2].url.as_deref(), Some("https://ci/lint"));
    }

    #[test]
    fn pr_meta_handles_nulls_and_empties() {
        let json = r#"{
          "repository": {
            "pullRequest": {
              "number": 1,
              "title": "t",
              "url": "u",
              "body": null,
              "state": "CLOSED",
              "isDraft": true,
              "mergeable": "UNKNOWN",
              "reviewDecision": null,
              "additions": 0,
              "deletions": 0,
              "changedFiles": 0,
              "author": null,
              "labels": { "nodes": [] },
              "latestReviews": { "nodes": [] },
              "commits": { "nodes": [] }
            }
          }
        }"#;
        let data: PrMetaData = serde_json::from_str(json).expect("parses");
        let meta = map_pr_meta(data.repository.unwrap().pull_request.unwrap());
        assert_eq!(meta.body, "");
        assert!(meta.is_draft);
        assert_eq!(meta.mergeable.as_deref(), Some("UNKNOWN"));
        assert!(meta.author.is_none());
        assert!(meta.labels.is_empty());
        assert!(meta.reviews.is_empty());
        assert_eq!(meta.ci_state, None);
        assert!(meta.checks.is_empty());
    }

    const THREADS_FIXTURE: &str = r#"{
      "repository": {
        "pullRequest": {
          "reviewThreads": {
            "pageInfo": { "hasNextPage": false, "endCursor": null },
            "nodes": [
              {
                "id": "T_resolved",
                "isResolved": true,
                "isOutdated": false,
                "isCollapsed": true,
                "path": "src/a.rs",
                "line": 12,
                "startLine": null,
                "originalLine": 12,
                "diffSide": "RIGHT",
                "startDiffSide": null,
                "subjectType": "LINE",
                "comments": { "pageInfo": { "hasNextPage": false, "endCursor": null }, "nodes": [
                  {
                    "id": "C1",
                    "databaseId": 1001,
                    "author": { "login": "rev1", "avatarUrl": "https://example.com/r1.png" },
                    "body": "Looks good to **me**.",
                    "createdAt": "2024-03-01T00:00:00Z",
                    "url": "https://github.com/acme/widget/pull/1#discussion_r1001",
                    "diffHunk": "@@ -1 +1 @@",
                    "outdated": false
                  }
                ] }
              },
              {
                "id": "T_outdated",
                "isResolved": false,
                "isOutdated": true,
                "isCollapsed": false,
                "path": "src/b.rs",
                "line": null,
                "startLine": null,
                "originalLine": 7,
                "diffSide": "RIGHT",
                "startDiffSide": null,
                "subjectType": "LINE",
                "comments": { "pageInfo": { "hasNextPage": false, "endCursor": null }, "nodes": [
                  {
                    "id": "C2",
                    "databaseId": 1002,
                    "author": { "login": "rev2", "avatarUrl": null },
                    "body": "This moved.",
                    "createdAt": "2024-03-02T00:00:00Z",
                    "url": "https://github.com/acme/widget/pull/1#discussion_r1002",
                    "diffHunk": null,
                    "outdated": true
                  },
                  {
                    "id": "C3",
                    "databaseId": 1003,
                    "author": null,
                    "body": "Agreed.",
                    "createdAt": "2024-03-03T00:00:00Z",
                    "url": "https://github.com/acme/widget/pull/1#discussion_r1003",
                    "diffHunk": null,
                    "outdated": true
                  }
                ] }
              }
            ]
          }
        }
      }
    }"#;

    #[test]
    fn pr_threads_maps_fixture() {
        let data: PrThreadsData = serde_json::from_str(THREADS_FIXTURE).expect("fixture parses");
        let conn = data.repository.unwrap().pull_request.unwrap().review_threads;
        assert!(!conn.page_info.has_next_page);
        let threads: Vec<PrThread> = conn.nodes.into_iter().map(map_thread).collect();

        assert_eq!(threads.len(), 2);

        let resolved = &threads[0];
        assert!(resolved.is_resolved);
        assert!(!resolved.is_outdated);
        assert_eq!(resolved.path.as_deref(), Some("src/a.rs"));
        assert_eq!(resolved.line, Some(12));
        assert_eq!(resolved.diff_side.as_deref(), Some("RIGHT"));
        assert_eq!(resolved.comments.len(), 1);
        assert_eq!(resolved.comments[0].body, "Looks good to **me**.");
        assert_eq!(resolved.comments[0].database_id, Some(1001));
        assert_eq!(
            resolved.comments[0].author.as_ref().and_then(|a| a.login.as_deref()),
            Some("rev1")
        );

        let outdated = &threads[1];
        assert!(outdated.is_outdated);
        assert!(!outdated.is_resolved);
        assert_eq!(outdated.line, None);
        assert_eq!(outdated.original_line, Some(7));
        // root + reply preserved in order.
        assert_eq!(outdated.comments.len(), 2);
        assert_eq!(outdated.comments[0].id, "C2");
        assert!(outdated.comments[0].outdated);
        assert_eq!(outdated.comments[1].id, "C3");
        assert!(outdated.comments[1].author.is_none());
    }

    const COMPARE_FIXTURE: &str = r#"{
      "status": "ahead",
      "ahead_by": 2,
      "merge_base_commit": { "sha": "abc123" },
      "files": [
        {
          "filename": "src/a.rs",
          "status": "modified",
          "additions": 3,
          "deletions": 1,
          "changes": 4,
          "patch": "@@ -1,2 +1,4 @@\n alpha\n+INSERTED\n beta\n-gamma\n+GAMMA"
        },
        {
          "filename": "assets/logo.png",
          "status": "added"
        }
      ]
    }"#;

    #[test]
    fn compare_parses_fixture() {
        let raw: CompareRaw = serde_json::from_str(COMPARE_FIXTURE).expect("fixture parses");
        assert_eq!(raw.files.len(), 2);

        let a = &raw.files[0];
        assert_eq!(a.filename, "src/a.rs");
        assert_eq!(a.status, "modified");
        assert!(a.patch.as_deref().unwrap().contains("+INSERTED"));

        // Binary/added file with no patch -> None via #[serde(default)].
        let logo = &raw.files[1];
        assert_eq!(logo.filename, "assets/logo.png");
        assert_eq!(logo.status, "added");
        assert!(logo.patch.is_none());
    }

    #[test]
    fn compare_fixture_parses_merge_base() {
        let raw: CompareRaw = serde_json::from_str(COMPARE_FIXTURE).expect("fixture parses");
        assert_eq!(raw.merge_base_commit.unwrap().sha, "abc123");
        // `files` still parse exactly as before alongside the new field.
        assert_eq!(raw.files.len(), 2);
        assert_eq!(raw.files[0].filename, "src/a.rs");
    }

    #[test]
    fn compare_fixture_without_merge_base_is_none() {
        // The #[serde(default)] contract compare() relies on: older/partial
        // responses without merge_base_commit still deserialize.
        let json = r#"{
          "status": "ahead",
          "files": [
            { "filename": "src/a.rs", "status": "modified" }
          ]
        }"#;
        let raw: CompareRaw = serde_json::from_str(json).expect("parses");
        assert!(raw.merge_base_commit.is_none());
        assert_eq!(raw.files.len(), 1);
    }

    const REVIEW_COMMENTS_FIXTURE: &str = r#"[
      {
        "id": 9001,
        "path": "src/lib.rs",
        "side": "RIGHT",
        "line": 5,
        "start_line": 3,
        "body": "ranged note"
      },
      {
        "id": 9002,
        "path": "src/lib.rs",
        "side": null,
        "line": null,
        "start_line": null,
        "body": ""
      }
    ]"#;

    #[test]
    fn review_comments_parse_fixture() {
        let comments: Vec<ReviewComment> =
            serde_json::from_str(REVIEW_COMMENTS_FIXTURE).expect("fixture parses");
        assert_eq!(comments.len(), 2);

        let full = &comments[0];
        assert_eq!(full.id, 9001);
        assert_eq!(full.path, "src/lib.rs");
        assert_eq!(full.side.as_deref(), Some("RIGHT"));
        assert_eq!(full.line, Some(5));
        assert_eq!(full.start_line, Some(3));
        assert_eq!(full.body, "ranged note");

        // Minimal item: null anchor fields land as None, body defaults to "".
        let minimal = &comments[1];
        assert_eq!(minimal.id, 9002);
        assert!(minimal.side.is_none());
        assert!(minimal.line.is_none());
        assert!(minimal.start_line.is_none());
        assert_eq!(minimal.body, "");
    }
}
