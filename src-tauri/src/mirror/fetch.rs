//! GraphQL side of the mirror: the one fragment every GitHub read shares
//! (inbox searches, board refetches, review targets, the PR header), the node
//! shape it deserializes into, and the batched fetchers. No DB access.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::json;

use crate::error::{AppError, AppResult};
use crate::gh;

/// `nodes(ids:)` with the full fragment takes GitHub several seconds per
/// batch; past ~10 s it answers 502, so batches stay small and run in parallel.
const REFETCH_BATCH_SIZE: usize = 25;
const REFETCH_CONCURRENCY: usize = 4;
const SEARCH_PAGE: i64 = 25;

/// `IssueCore` / `PrCore`, appended to every mirror query. The PR fragment
/// covers the union of what Inbox, Board and Review display.
pub const FRAGMENTS: &str = r#"
fragment IssueCore on Issue {
  id number title url bodyText state updatedAt closedAt
  author { login avatarUrl }
  repository { nameWithOwner }
  comments(last: 1) { nodes { bodyText author { login } } }
}
fragment PrCore on PullRequest {
  id number title url bodyText state isDraft merged updatedAt closedAt
  baseRefName headRefName headRefOid
  author { login avatarUrl }
  repository { nameWithOwner }
  changedFiles additions deletions reviewDecision
  reviews(first: 1) { totalCount }
  comments(last: 1) { nodes { bodyText author { login } } }
  mergeable mergeStateStatus viewerCanUpdateBranch
  autoMergeRequest { mergeMethod enabledBy { login } }
  stack { number }
  stackEntry { position }
  commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
  closingIssuesReferences(first: 20) { nodes { id number repository { nameWithOwner } } }
}
"#;

const SELECTION: &str = "__typename ...IssueCore ...PrCore";

fn with_fragments(query: String) -> String {
    format!("{query}\n{FRAGMENTS}")
}

// ---------------------------------------------------------------------------
// Node shape
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Actor {
    #[serde(default)]
    pub login: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoRef {
    pub name_with_owner: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TotalCount {
    pub total_count: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CommentConn {
    #[serde(default)]
    pub nodes: Vec<CommentNode>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentNode {
    #[serde(default)]
    pub body_text: Option<String>,
    #[serde(default)]
    pub author: Option<Actor>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoMergeRef {
    #[serde(default)]
    pub merge_method: Option<String>,
    #[serde(default)]
    pub enabled_by: Option<Actor>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StackRef {
    pub number: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StackEntryRef {
    pub position: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RollupRef {
    #[serde(default)]
    pub state: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitInner {
    #[serde(default)]
    pub status_check_rollup: Option<RollupRef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommitNode {
    pub commit: CommitInner,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CommitConn {
    #[serde(default)]
    pub nodes: Vec<CommitNode>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClosingIssue {
    pub id: String,
    #[allow(dead_code)]
    pub number: i64,
    #[allow(dead_code)]
    pub repository: RepoRef,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ClosingIssuesConn {
    #[serde(default)]
    pub nodes: Vec<ClosingIssue>,
}

/// One Issue or PullRequest as returned by `IssueCore` / `PrCore`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    #[serde(rename = "__typename")]
    pub typename: String,
    pub id: String,
    pub number: i64,
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub body_text: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub is_draft: bool,
    #[serde(default)]
    pub merged: bool,
    pub updated_at: String,
    #[serde(default)]
    pub closed_at: Option<String>,
    #[serde(default)]
    pub base_ref_name: Option<String>,
    #[serde(default)]
    pub head_ref_name: Option<String>,
    #[serde(default)]
    pub head_ref_oid: Option<String>,
    #[serde(default)]
    pub author: Option<Actor>,
    #[serde(default)]
    pub repository: Option<RepoRef>,
    #[serde(default)]
    pub changed_files: Option<i64>,
    #[serde(default)]
    pub additions: Option<i64>,
    #[serde(default)]
    pub deletions: Option<i64>,
    #[serde(default)]
    pub review_decision: Option<String>,
    #[serde(default)]
    pub reviews: Option<TotalCount>,
    #[serde(default)]
    pub comments: CommentConn,
    #[serde(default)]
    pub mergeable: Option<String>,
    #[serde(default)]
    pub merge_state_status: Option<String>,
    #[serde(default)]
    pub viewer_can_update_branch: bool,
    #[serde(default)]
    pub auto_merge_request: Option<AutoMergeRef>,
    #[serde(default)]
    pub stack: Option<StackRef>,
    #[serde(default)]
    pub stack_entry: Option<StackEntryRef>,
    #[serde(default)]
    pub commits: Option<CommitConn>,
    #[serde(default)]
    pub closing_issues_references: Option<ClosingIssuesConn>,
}

impl Node {
    pub fn is_pr(&self) -> bool {
        self.typename == "PullRequest"
    }

    fn is_supported(&self) -> bool {
        self.typename == "PullRequest" || self.typename == "Issue"
    }

    /// `owner/name`, from GitHub's `nameWithOwner`.
    pub fn repo(&self) -> Option<&str> {
        self.repository.as_ref().map(|r| r.name_with_owner.as_str())
    }

    pub fn owner_name(&self) -> Option<(&str, &str)> {
        self.repo()?.split_once('/')
    }

    /// `open` / `closed` / `merged`: a merged PR is `merged`, not GitHub's `CLOSED`.
    pub fn normalized_state(&self) -> Option<String> {
        if self.is_pr() && self.merged {
            Some("merged".to_string())
        } else {
            self.state.as_ref().map(|s| s.to_lowercase())
        }
    }

    pub fn is_closed(&self) -> bool {
        matches!(self.normalized_state().as_deref(), Some("closed") | Some("merged"))
    }

    /// The head commit's combined CI state, in GitHub casing (`SUCCESS`, ...).
    pub fn ci_state(&self) -> Option<String> {
        self.commits
            .as_ref()
            .and_then(|c| c.nodes.first())
            .and_then(|n| n.commit.status_check_rollup.as_ref())
            .and_then(|r| r.state.clone())
    }

    /// Merge method of an enabled auto-merge (`SQUASH`/`MERGE`/`REBASE`).
    pub fn auto_merge_method(&self) -> Option<String> {
        self.auto_merge_request
            .as_ref()
            .map(|r| r.merge_method.clone().unwrap_or_else(|| "MERGE".into()))
    }

    pub fn author_login(&self) -> Option<&str> {
        self.author.as_ref().and_then(|a| a.login.as_deref())
    }

    pub fn latest_comment(&self) -> Option<&CommentNode> {
        self.comments.nodes.first()
    }
}

// ---------------------------------------------------------------------------
// Fetchers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct NodesData {
    nodes: Vec<Option<Node>>,
}

#[derive(Debug, Deserialize)]
struct SearchData {
    search: SearchConn,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchConn {
    #[serde(default)]
    page_info: Option<PageInfo>,
    nodes: Vec<Node>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResolveData {
    repository: Option<ResolveRepo>,
}

#[derive(Debug, Deserialize)]
struct ResolveRepo {
    #[serde(rename = "issueOrPullRequest")]
    issue_or_pull_request: Option<Node>,
}

/// Up to `limit` `search(type: ISSUE)` results, PRs and issues only, in
/// pages small enough to stay under GitHub's timeout. A failed page is retried
/// once; a failure after the first page keeps what was already fetched.
pub fn search(q: &str, limit: i64) -> AppResult<Vec<Node>> {
    let query = with_fragments(format!(
        "query MirrorSearch($q: String!, $first: Int!, $after: String) {{\n  search(query: $q, type: ISSUE, first: $first, after: $after) {{ pageInfo {{ hasNextPage endCursor }} nodes {{ {SELECTION} }} }}\n}}"
    ));
    let mut nodes = Vec::new();
    let mut after: Option<String> = None;
    while (nodes.len() as i64) < limit {
        let vars = json!({ "q": q, "first": SEARCH_PAGE.min(limit - nodes.len() as i64), "after": after });
        let page = gh::graphql::<SearchData>(&query, vars.clone()).or_else(|e| {
            crate::logging::warn(&format!("[mirror.search] q={q:?} page failed, retrying: {e}"));
            gh::graphql::<SearchData>(&query, vars)
        });
        let page = match page {
            Ok(page) => page.search,
            Err(e) if nodes.is_empty() => return Err(e),
            Err(_) => break,
        };
        nodes.extend(page.nodes.into_iter().filter(Node::is_supported));
        match page.page_info.filter(|p| p.has_next_page).and_then(|p| p.end_cursor) {
            Some(cursor) => after = Some(cursor),
            None => break,
        }
    }
    Ok(nodes)
}

/// One `nodes(ids:)` batch. A failed batch (usually a GitHub timeout) is
/// split in half and retried once; ids that still fail are left out.
fn refetch_batch(query: &str, ids: &[String], may_split: bool) -> HashMap<String, Option<Node>> {
    match gh::graphql::<NodesData>(query, json!({ "ids": ids })) {
        Ok(data) => {
            let mut found: HashMap<String, Node> = data
                .nodes
                .into_iter()
                .flatten()
                .filter(Node::is_supported)
                .map(|n| (n.id.clone(), n))
                .collect();
            ids.iter().map(|id| (id.clone(), found.remove(id))).collect()
        }
        Err(e) if may_split && ids.len() > 1 => {
            crate::logging::warn(&format!("[mirror.refetch] batch of {} failed, splitting: {e}", ids.len()));
            let (a, b) = ids.split_at(ids.len() / 2);
            let mut out = refetch_batch(query, a, false);
            out.extend(refetch_batch(query, b, false));
            out
        }
        Err(e) => {
            crate::logging::warn(&format!("[mirror.refetch] batch of {} failed: {e}", ids.len()));
            HashMap::new()
        }
    }
}

/// Refetch nodes by id, in concurrent batches. Every id of a successful batch
/// is present: `None` when GitHub no longer returns it (deleted or
/// inaccessible). Ids of a failed batch are absent, since their state is unknown.
pub fn refetch(ids: &[String]) -> HashMap<String, Option<Node>> {
    let query = with_fragments(format!(
        "query MirrorNodes($ids: [ID!]!) {{\n  nodes(ids: $ids) {{ {SELECTION} }}\n}}"
    ));
    let chunks: Vec<&[String]> = ids.chunks(REFETCH_BATCH_SIZE).collect();
    crate::inbox::concurrent_map(chunks, REFETCH_CONCURRENCY, |chunk| refetch_batch(&query, chunk, true))
        .into_iter()
        .flatten()
        .collect()
}

/// Resolve an issue or PR by `owner/name#number`.
pub fn resolve(owner: &str, name: &str, number: i64) -> AppResult<Node> {
    let query = with_fragments(format!(
        "query MirrorResolve($owner: String!, $name: String!, $number: Int!) {{\n  repository(owner: $owner, name: $name) {{ issueOrPullRequest(number: $number) {{ {SELECTION} }} }}\n}}"
    ));
    let data: ResolveData = gh::graphql(&query, json!({ "owner": owner, "name": name, "number": number }))?;
    data.repository
        .and_then(|r| r.issue_or_pull_request)
        .filter(Node::is_supported)
        .ok_or_else(|| AppError::Other(format!("{owner}/{name}#{number} not found")))
}

/// Resolve a PR by `owner/name#number`, rejecting issues.
pub fn resolve_pr(owner: &str, name: &str, number: i64) -> AppResult<Node> {
    let node = resolve(owner, name, number)?;
    if !node.is_pr() {
        return Err(AppError::Other(format!("{owner}/{name}#{number} is not a pull request")));
    }
    Ok(node)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PR_NODE_FIXTURE: &str = r#"{
      "__typename": "PullRequest",
      "id": "PR_1",
      "number": 42,
      "title": "t",
      "url": "u",
      "bodyText": "body",
      "state": "OPEN",
      "isDraft": false,
      "merged": false,
      "updatedAt": "2024-01-01T00:00:00Z",
      "closedAt": null,
      "baseRefName": "main",
      "headRefName": "pbq/issue-7",
      "headRefOid": "abc123",
      "author": { "login": "pl-buiquang", "avatarUrl": "https://a/x.png" },
      "repository": { "nameWithOwner": "acme/widget" },
      "changedFiles": 3, "additions": 10, "deletions": 2,
      "reviewDecision": "REVIEW_REQUIRED",
      "reviews": { "totalCount": 2 },
      "comments": { "nodes": [ { "bodyText": "hi", "author": { "login": "rev" } } ] },
      "mergeable": "MERGEABLE",
      "mergeStateStatus": "BEHIND",
      "viewerCanUpdateBranch": true,
      "autoMergeRequest": { "mergeMethod": "SQUASH", "enabledBy": { "login": "me" } },
      "stack": { "number": 40 },
      "stackEntry": { "position": 2 },
      "commits": { "nodes": [ { "commit": { "statusCheckRollup": { "state": "SUCCESS" } } } ] },
      "closingIssuesReferences": { "nodes": [ { "id": "I_1", "number": 7, "repository": { "nameWithOwner": "acme/widget" } } ] }
    }"#;

    #[test]
    fn node_deserializes_a_real_pr_fixture() {
        let node: Node = serde_json::from_str(PR_NODE_FIXTURE).expect("fixture parses");
        assert!(node.is_pr());
        assert_eq!(node.owner_name(), Some(("acme", "widget")));
        assert_eq!(node.head_ref_name.as_deref(), Some("pbq/issue-7"));
        assert_eq!(node.head_ref_oid.as_deref(), Some("abc123"));
        assert_eq!(node.ci_state().as_deref(), Some("SUCCESS"));
        assert_eq!(node.reviews.as_ref().map(|r| r.total_count), Some(2));
        assert_eq!(node.auto_merge_method().as_deref(), Some("SQUASH"));
        assert_eq!(node.stack.as_ref().map(|s| s.number), Some(40));
        assert_eq!(node.latest_comment().and_then(|c| c.body_text.as_deref()), Some("hi"));
        assert_eq!(node.closing_issues_references.unwrap().nodes[0].id, "I_1");
    }

    #[test]
    fn merged_pr_normalizes_to_merged() {
        let mut node: Node = serde_json::from_str(PR_NODE_FIXTURE).unwrap();
        node.merged = true;
        node.state = Some("CLOSED".into());
        assert_eq!(node.normalized_state().as_deref(), Some("merged"));
        assert!(node.is_closed());
    }

    #[test]
    fn issue_without_pr_fields_deserializes() {
        let json = r#"{
          "__typename": "Issue", "id": "I_1", "number": 7, "title": "t", "url": "u",
          "state": "OPEN", "updatedAt": "2024-01-01T00:00:00Z", "author": null,
          "repository": { "nameWithOwner": "acme/widget" }, "comments": { "nodes": [] }
        }"#;
        let node: Node = serde_json::from_str(json).unwrap();
        assert!(!node.is_pr());
        assert_eq!(node.normalized_state().as_deref(), Some("open"));
        assert_eq!(node.ci_state(), None);
    }
}
