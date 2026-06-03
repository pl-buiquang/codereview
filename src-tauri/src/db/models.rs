use rusqlite::Row;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Repository {
    pub id: i64,
    pub path: String,
    pub remote_owner: Option<String>,
    pub remote_name: Option<String>,
    pub default_branch: Option<String>,
    pub added_at: String,
}

impl Repository {
    pub fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            path: row.get("path")?,
            remote_owner: row.get("remote_owner")?,
            remote_name: row.get("remote_name")?,
            default_branch: row.get("default_branch")?,
            added_at: row.get("added_at")?,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Target {
    pub id: i64,
    pub repo_id: i64,
    pub kind: String,
    pub github_pr_number: Option<i64>,
    pub title: String,
    pub base_ref: String,
    pub head_ref: String,
    pub base_sha: Option<String>,
    pub head_sha: Option<String>,
    pub three_dot: bool,
    pub created_at: String,
}

impl Target {
    pub fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            repo_id: row.get("repo_id")?,
            kind: row.get("kind")?,
            github_pr_number: row.get("github_pr_number")?,
            title: row.get("title")?,
            base_ref: row.get("base_ref")?,
            head_ref: row.get("head_ref")?,
            base_sha: row.get("base_sha")?,
            head_sha: row.get("head_sha")?,
            three_dot: row.get::<_, i64>("three_dot")? != 0,
            created_at: row.get("created_at")?,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Review {
    pub id: i64,
    pub target_id: i64,
    pub body: String,
    pub event: Option<String>,
    pub status: String,
    pub published_at: Option<String>,
    pub github_review_id: Option<i64>,
    pub last_exported_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Review {
    pub fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            target_id: row.get("target_id")?,
            body: row.get("body")?,
            event: row.get("event")?,
            status: row.get("status")?,
            published_at: row.get("published_at")?,
            github_review_id: row.get("github_review_id")?,
            last_exported_at: row.get("last_exported_at")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Comment {
    pub id: i64,
    pub review_id: i64,
    pub file_path: String,
    pub side: String,
    pub line: i64,
    pub start_line: Option<i64>,
    pub diff_hunk: Option<String>,
    pub body: String,
    pub parent_id: Option<i64>,
    pub anchored_head_sha: Option<String>,
    pub github_comment_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

impl Comment {
    pub fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            review_id: row.get("review_id")?,
            file_path: row.get("file_path")?,
            side: row.get("side")?,
            line: row.get("line")?,
            start_line: row.get("start_line")?,
            diff_hunk: row.get("diff_hunk")?,
            body: row.get("body")?,
            parent_id: row.get("parent_id")?,
            anchored_head_sha: row.get("anchored_head_sha")?,
            github_comment_id: row.get("github_comment_id")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }
}

/// A review plus its target/repo context and inline comments — the payload the
/// Review screen loads to reconstruct full state.
#[derive(Debug, Serialize)]
pub struct ReviewDetail {
    pub review: Review,
    pub target: Target,
    pub repo_path: String,
    pub comments: Vec<Comment>,
}

/// Row for the global Reviews list.
#[derive(Debug, Serialize)]
pub struct ReviewSummary {
    pub review: Review,
    pub target: Target,
    pub repo_id: i64,
    pub repo_label: String,
    pub comment_count: i64,
}
