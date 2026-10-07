//! Read-only git history for the Graph tab (spec 22). Types here are mirrored in `src/lib/types.ts`.

pub mod branches;
pub mod commit;
pub mod log;
pub mod wip;

use serde::{Deserialize, Serialize};

pub use branches::{delete_gone_branches, gone_branches};
pub use commit::{commit_detail, commit_file_diff};
pub use log::{graph_log, list_refs};
pub use wip::{worktree_file_diff, worktree_status};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    All,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WipKind {
    Staged,
    Unstaged,
    Untracked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    Local,
    Remote,
    Tag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeStatus {
    #[serde(rename = "A")]
    Added,
    #[serde(rename = "M")]
    Modified,
    #[serde(rename = "D")]
    Deleted,
    #[serde(rename = "R")]
    Renamed,
    #[serde(rename = "C")]
    Copied,
    #[serde(rename = "T")]
    TypeChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphCommit {
    pub sha: String,
    pub parents: Vec<String>,
    pub subject: String,
    /// First line of the body after the subject, `""` if none.
    pub body_preview: String,
    pub author_name: String,
    pub author_email: String,
    /// Unix seconds.
    pub author_time: i64,
    pub committer_time: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RefInfo {
    /// Short name, e.g. "main", "origin/main", "v1.2.0".
    pub name: String,
    pub kind: RefKind,
    /// Peeled commit sha for annotated tags.
    pub sha: String,
    pub is_head: bool,
    /// Unix seconds: tip commit date, or tagger date for annotated tags.
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitDetail {
    pub sha: String,
    pub parents: Vec<String>,
    pub subject: String,
    pub body: String,
    pub author_name: String,
    pub author_email: String,
    pub author_time: i64,
    pub committer_name: String,
    pub committer_email: String,
    pub committer_time: i64,
    pub files: Vec<ChangedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangedFile {
    pub path: String,
    /// `Some` only for renames/copies.
    pub old_path: Option<String>,
    pub status: ChangeStatus,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WipStatus {
    pub head_sha: String,
    pub staged: Vec<ChangedFile>,
    pub unstaged: Vec<ChangedFile>,
    pub untracked: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_serialize_to_spec_literals() {
        assert_eq!(serde_json::to_string(&RefKind::Remote).unwrap(), "\"remote\"");
        assert_eq!(serde_json::to_string(&ChangeStatus::Renamed).unwrap(), "\"R\"");
        assert_eq!(serde_json::to_string(&ChangeStatus::TypeChanged).unwrap(), "\"T\"");
        let s: Scope = serde_json::from_str("\"local\"").unwrap();
        assert_eq!(s, Scope::Local);
        let k: WipKind = serde_json::from_str("\"untracked\"").unwrap();
        assert_eq!(k, WipKind::Untracked);
    }
}
