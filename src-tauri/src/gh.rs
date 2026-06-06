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
