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
    /// 'line' (anchored to side/line) or 'file' (attached to the whole file).
    pub subject_type: String,
    /// 'diff' (authored against a diff hunk; publishable inline) or 'file_view'
    /// (authored in the full-file pane; folded into the review body on publish).
    pub origin: String,
    pub side: String,
    pub line: i64,
    pub start_line: Option<i64>,
    pub diff_hunk: Option<String>,
    pub body: String,
    pub parent_id: Option<i64>,
    pub anchored_head_sha: Option<String>,
    pub anchored_base_sha: Option<String>,
    pub github_comment_id: Option<i64>,
    pub resolved_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Comment {
    pub fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            review_id: row.get("review_id")?,
            file_path: row.get("file_path")?,
            subject_type: row.get("subject_type")?,
            origin: row.get("origin")?,
            side: row.get("side")?,
            line: row.get("line")?,
            start_line: row.get("start_line")?,
            diff_hunk: row.get("diff_hunk")?,
            body: row.get("body")?,
            parent_id: row.get("parent_id")?,
            anchored_head_sha: row.get("anchored_head_sha")?,
            anchored_base_sha: row.get("anchored_base_sha")?,
            github_comment_id: row.get("github_comment_id")?,
            resolved_at: row.get("resolved_at")?,
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
    /// GitHub `owner`/`name` of the target's repo, if it has a remote — used to
    /// build the PR's web URL for the "Open PR" action.
    pub remote_owner: Option<String>,
    pub remote_name: Option<String>,
    pub comments: Vec<Comment>,
    /// File paths the user has marked "viewed" (collapsed) for this review.
    pub viewed_files: Vec<String>,
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

/// A GitHub inbox item (PR or issue needing attention). Mirrors the `items`
/// table; serialized to the frontend with the DB column names.
#[derive(Debug, Serialize, Deserialize)]
pub struct ItemRow {
    pub id: String,
    #[serde(rename = "type")]
    pub typ: String,
    pub number: i64,
    pub repo: String,
    pub title: String,
    pub url: String,
    pub author_login: Option<String>,
    pub author_avatar: Option<String>,
    pub state: Option<String>,
    pub is_draft: bool,
    pub body: Option<String>,
    pub latest_comment: Option<String>,
    pub latest_actor: Option<String>,
    pub updated_at: String,
    pub files_changed: Option<i64>,
    pub additions: Option<i64>,
    pub deletions: Option<i64>,
    pub top_files_json: Option<String>,
    pub ci_state: Option<String>,
    pub review_decision: Option<String>,
    pub untracked_at: Option<String>,
    pub closed_at: Option<String>,
    pub engaged_at: Option<String>,
    pub first_seen_at: String,
    pub last_refreshed: String,
}

impl ItemRow {
    pub fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            typ: row.get("type")?,
            number: row.get("number")?,
            repo: row.get("repo")?,
            title: row.get("title")?,
            url: row.get("url")?,
            author_login: row.get("author_login")?,
            author_avatar: row.get("author_avatar")?,
            state: row.get("state")?,
            is_draft: row.get::<_, i64>("is_draft")? != 0,
            body: row.get("body")?,
            latest_comment: row.get("latest_comment")?,
            latest_actor: row.get("latest_actor")?,
            updated_at: row.get("updated_at")?,
            files_changed: row.get("files_changed")?,
            additions: row.get("additions")?,
            deletions: row.get("deletions")?,
            top_files_json: row.get("top_files_json")?,
            ci_state: row.get("ci_state")?,
            review_decision: row.get("review_decision")?,
            untracked_at: row.get("untracked_at")?,
            closed_at: row.get("closed_at")?,
            engaged_at: row.get("engaged_at")?,
            first_seen_at: row.get("first_seen_at")?,
            last_refreshed: row.get("last_refreshed")?,
        })
    }
}

/// One reason an item is in the inbox (e.g. `assigned`, `team_review` with a
/// `org/team` detail).
#[derive(Debug, Serialize)]
pub struct ItemReason {
    pub reason: String,
    pub detail: String,
}

/// An inbox item plus the reasons it surfaced; the frontend buckets on these.
#[derive(Debug, Serialize)]
pub struct ItemWithReasons {
    #[serde(flatten)]
    pub item: ItemRow,
    pub reasons: Vec<ItemReason>,
}
