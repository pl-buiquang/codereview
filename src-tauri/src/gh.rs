use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// Run a `gh` command with the repo as the working directory (so `gh` resolves
/// the GitHub repo from its `origin` remote). Returns stdout on success.
pub fn run_gh(repo: &Path, args: &[&str]) -> AppResult<String> {
    let output = Command::new(crate::tools::gh_bin())
        .current_dir(repo)
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
pub fn run_gh_stdin(repo: &Path, args: &[&str], input: &str) -> AppResult<String> {
    let mut child = Command::new(crate::tools::gh_bin())
        .current_dir(repo)
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

/// POST a review (body + verdict + inline comments) to a PR. Returns the new
/// GitHub review id.
pub fn post_review(
    repo: &Path,
    owner: &str,
    name: &str,
    number: i64,
    payload_json: &str,
) -> AppResult<i64> {
    let endpoint = format!("repos/{owner}/{name}/pulls/{number}/reviews");
    let out = run_gh_stdin(
        repo,
        &["api", &endpoint, "--method", "POST", "--input", "-"],
        payload_json,
    )?;
    let value: serde_json::Value = serde_json::from_str(&out)
        .map_err(|e| AppError::Gh(format!("failed to parse review response: {e}")))?;
    Ok(value.get("id").and_then(|v| v.as_i64()).unwrap_or_default())
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

pub fn list_prs(repo: &Path) -> AppResult<Vec<PrSummary>> {
    let out = run_gh(
        repo,
        &[
            "pr",
            "list",
            "--state",
            "open",
            "--limit",
            "100",
            "--json",
            "number,title,author,headRefName,baseRefName,createdAt,url",
        ],
    )?;
    serde_json::from_str(&out).map_err(|e| AppError::Gh(format!("failed to parse pr list: {e}")))
}

pub fn pr_diff(repo: &Path, number: i64) -> AppResult<String> {
    run_gh(repo, &["pr", "diff", &number.to_string()])
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

pub fn pr_view(repo: &Path, number: i64) -> AppResult<PrInfo> {
    let out = run_gh(
        repo,
        &[
            "pr",
            "view",
            &number.to_string(),
            "--json",
            "title,baseRefName,headRefName,headRefOid",
        ],
    )?;
    let raw: PrViewRaw =
        serde_json::from_str(&out).map_err(|e| AppError::Gh(format!("failed to parse pr view: {e}")))?;
    Ok(PrInfo {
        title: raw.title,
        base_ref: raw.base_ref_name,
        head_ref: raw.head_ref_name,
        head_sha: raw.head_ref_oid,
    })
}
