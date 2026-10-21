//! GraphQL side of the mirror: the one fragment every GitHub read shares
//! (inbox searches, board refetches, review targets, the PR header), the node
//! shape it deserializes into, and the batched fetchers. No DB access.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{AppError, AppResult};
use crate::gh;

/// `nodes(ids:)` with the full fragment takes GitHub several seconds per
/// batch; past ~10 s it answers 502, so batches stay small and run in parallel.
const REFETCH_BATCH_SIZE: usize = 25;
const REFETCH_CONCURRENCY: usize = 4;
const SEARCH_PAGE: i64 = 25;

macro_rules! issue_core {
    () => {
        r#"
fragment IssueCore on Issue {
  id number title url bodyText state updatedAt closedAt
  author { __typename login avatarUrl }
  repository { nameWithOwner }
  comments(last: 1) { nodes { bodyText author { login } } }
}
"#
    };
}

macro_rules! pr_core {
    () => {
        r#"
fragment PrCore on PullRequest {
  id number title url bodyText state isDraft merged updatedAt closedAt
  baseRefName headRefName headRefOid
  author { __typename login avatarUrl }
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
"#
    };
}

/// `IssueCore` / `PrCore`, appended to every mirror query. The PR fragment
/// covers the union of what Inbox, Board and Review display.
pub const FRAGMENTS: &str = concat!(issue_core!(), pr_core!());
/// For PR-only queries: GitHub rejects a query with an unused fragment.
pub const PR_FRAGMENT: &str = pr_core!();
/// Branches per by-head lookup query (each alias returns up to 3 PRs).
const HEAD_LOOKUP_BATCH: usize = 8;

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
    #[serde(default, rename = "__typename")]
    pub type_name: Option<String>,
    #[serde(default)]
    pub login: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

impl Actor {
    /// GitHub reports the author as a `Bot` (dependabot, renovate, apps).
    pub fn is_bot(&self) -> bool {
        self.type_name.as_deref() == Some("Bot")
    }
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

/// A `nodes(ids:)` result node: a PR/issue `T`, or another type (no `id`).
trait Fetched: serde::de::DeserializeOwned {
    fn supported_id(&self) -> Option<&str>;
}

impl Fetched for Node {
    fn supported_id(&self) -> Option<&str> {
        self.is_supported().then_some(self.id.as_str())
    }
}

#[derive(Debug, Deserialize)]
#[serde(bound = "T: serde::de::DeserializeOwned")]
struct NodesOf<T> {
    nodes: Vec<Option<T>>,
}

/// One `nodes(ids:)` batch. A failed batch (usually a GitHub timeout) is
/// split in half and retried once; ids that still fail are left out.
fn nodes_batch<T: Fetched>(query: &str, ids: &[String], may_split: bool) -> HashMap<String, Option<T>> {
    match gh::graphql::<NodesOf<T>>(query, json!({ "ids": ids })) {
        Ok(data) => {
            let mut found: HashMap<String, T> = data
                .nodes
                .into_iter()
                .flatten()
                .filter_map(|n| Some((n.supported_id()?.to_string(), n)))
                .collect();
            ids.iter().map(|id| (id.clone(), found.remove(id))).collect()
        }
        Err(e) if may_split && ids.len() > 1 => {
            crate::logging::warn(&format!("[mirror.nodes] batch of {} failed, splitting: {e}", ids.len()));
            let (a, b) = ids.split_at(ids.len() / 2);
            let mut out = nodes_batch(query, a, false);
            out.extend(nodes_batch(query, b, false));
            out
        }
        Err(e) => {
            crate::logging::warn(&format!("[mirror.nodes] batch of {} failed: {e}", ids.len()));
            HashMap::new()
        }
    }
}

/// Every id of a successful batch is present: `None` when GitHub no longer
/// returns it (deleted or inaccessible). Ids of a failed batch are absent,
/// since their state is unknown.
fn nodes_by_id<T: Fetched + Send>(query: &str, ids: &[String], batch: usize) -> HashMap<String, Option<T>> {
    let chunks: Vec<&[String]> = ids.chunks(batch).collect();
    crate::inbox::concurrent_map(chunks, REFETCH_CONCURRENCY, |chunk| nodes_batch(query, chunk, true))
        .into_iter()
        .flatten()
        .collect()
}

/// Refetch nodes by id with the full fragment, in concurrent batches.
pub fn refetch(ids: &[String]) -> HashMap<String, Option<Node>> {
    let query = with_fragments(format!(
        "query MirrorNodes($ids: [ID!]!) {{\n  nodes(ids: $ids) {{ {SELECTION} }}\n}}"
    ));
    nodes_by_id(&query, ids, REFETCH_BATCH_SIZE)
}

// ---------------------------------------------------------------------------
// Probe
// ---------------------------------------------------------------------------

/// `mergeStateStatus`, `reviewDecision` and the CI rollup cost GitHub ~100 ms
/// per open PR, so the full probe runs in small parallel batches; the cheap
/// one (`updatedAt` + `state`) fits 100 ids in about a second.
const PROBE_BATCH_SIZE: usize = 20;
const CHEAP_PROBE_BATCH_SIZE: usize = 100;

const PROBE_QUERY: &str = r#"
query MirrorProbe($ids: [ID!]!) {
  nodes(ids: $ids) {
    __typename
    ... on PullRequest {
      id updatedAt headRefOid state merged isDraft reviewDecision mergeStateStatus
      autoMergeRequest { mergeMethod }
      commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
    }
    ... on Issue { id updatedAt state }
  }
}
"#;

const CHEAP_PROBE_QUERY: &str = r#"
query MirrorProbeCheap($ids: [ID!]!) {
  nodes(ids: $ids) {
    __typename
    ... on PullRequest { id updatedAt state merged }
    ... on Issue { id updatedAt state }
  }
}
"#;

/// The change keys of a PR or issue: cheap to fetch, and enough to decide
/// whether the full fragment needs refetching. `updatedAt` alone misses CI and
/// mergeability changes, hence the extra fields.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    #[serde(rename = "__typename")]
    pub typename: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub head_ref_oid: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub merged: bool,
    #[serde(default)]
    pub is_draft: bool,
    #[serde(default)]
    pub review_decision: Option<String>,
    #[serde(default)]
    pub merge_state_status: Option<String>,
    #[serde(default)]
    pub auto_merge_request: Option<AutoMergeRef>,
    #[serde(default)]
    pub commits: Option<CommitConn>,
    /// From the cheap probe: only `updatedAt` and `state` are meaningful.
    #[serde(skip)]
    pub cheap: bool,
}

impl Fetched for Probe {
    fn supported_id(&self) -> Option<&str> {
        matches!(self.typename.as_str(), "PullRequest" | "Issue").then_some(self.id.as_deref()).flatten()
    }
}

impl Probe {
    pub fn is_pr(&self) -> bool {
        self.typename == "PullRequest"
    }

    /// Same normalization as [`Node::normalized_state`].
    pub fn normalized_state(&self) -> Option<String> {
        if self.is_pr() && self.merged {
            Some("merged".to_string())
        } else {
            self.state.as_ref().map(|s| s.to_lowercase())
        }
    }

    pub fn ci_state(&self) -> Option<String> {
        self.commits
            .as_ref()
            .and_then(|c| c.nodes.first())
            .and_then(|n| n.commit.status_check_rollup.as_ref())
            .and_then(|r| r.state.clone())
    }

    /// Same default as [`Node::auto_merge_method`].
    pub fn auto_merge_method(&self) -> Option<String> {
        self.auto_merge_request
            .as_ref()
            .map(|r| r.merge_method.clone().unwrap_or_else(|| "MERGE".into()))
    }
}

/// Probe nodes by id, with the same presence semantics as [`refetch`]. `cheap`
/// ids (closed PRs, issues, ids not mirrored yet) only get `updatedAt` + `state`.
pub fn probe(full: &[String], cheap: &[String]) -> HashMap<String, Option<Probe>> {
    let mut out = nodes_by_id(PROBE_QUERY, full, PROBE_BATCH_SIZE);
    let cheap: HashMap<String, Option<Probe>> = nodes_by_id(CHEAP_PROBE_QUERY, cheap, CHEAP_PROBE_BATCH_SIZE);
    out.extend(cheap.into_iter().map(|(id, p)| (id, p.map(|p| Probe { cheap: true, ..p }))));
    out
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

fn prs_by_head_query(n: usize) -> String {
    let decls: String = (0..n).map(|i| format!(", $b{i}: String!")).collect();
    let fields: String = (0..n)
        .map(|i| {
            format!(
                "    b{i}: pullRequests(headRefName: $b{i}, first: 3, orderBy: {{field: CREATED_AT, direction: DESC}}) {{ nodes {{ __typename ...PrCore }} }}\n"
            )
        })
        .collect();
    format!(
        "query MirrorPrsByHead($owner: String!, $name: String!{decls}) {{\n  repository(owner: $owner, name: $name) {{\n{fields}  }}\n}}\n{PR_FRAGMENT}"
    )
}

#[derive(Debug, Deserialize)]
struct ByHeadData {
    repository: Option<HashMap<String, ByHeadConn>>,
}

#[derive(Debug, Deserialize)]
struct ByHeadConn {
    nodes: Vec<Node>,
}

/// The PRs (any state, newest 3 per branch) whose head branch is one of
/// `branches`, in concurrent batches. Also returns the branches whose batch
/// failed (their PRs are unknown, not absent).
pub fn prs_by_head(owner: &str, name: &str, branches: &[String]) -> (Vec<Node>, Vec<String>) {
    let chunks: Vec<&[String]> = branches.chunks(HEAD_LOOKUP_BATCH).collect();
    let results = crate::inbox::concurrent_map(chunks, REFETCH_CONCURRENCY, |chunk| {
        let mut vars = json!({ "owner": owner, "name": name });
        for (i, b) in chunk.iter().enumerate() {
            vars[format!("b{i}")] = json!(b);
        }
        (chunk.to_vec(), gh::graphql::<ByHeadData>(&prs_by_head_query(chunk.len()), vars))
    });
    let mut nodes = Vec::new();
    let mut failed = Vec::new();
    for (chunk, r) in results {
        match r {
            Ok(data) => nodes.extend(data.repository.into_iter().flat_map(|m| m.into_values()).flat_map(|c| c.nodes)),
            Err(e) => {
                crate::logging::warn(&format!("[mirror.by_head] {owner}/{name} batch failed: {e}"));
                failed.extend(chunk);
            }
        }
    }
    (nodes.into_iter().filter(Node::is_pr).collect(), failed)
}

// ---------------------------------------------------------------------------
// Discussion facet (top-level comments + review bodies)
// ---------------------------------------------------------------------------

const DISCUSSION_QUERY: &str = r#"
query MirrorDiscussion($ids: [ID!]!) {
  nodes(ids: $ids) {
    ... on PullRequest {
      id updatedAt
      comments(last: 50) { nodes { id databaseId author { __typename login avatarUrl } body createdAt url } }
      reviews(last: 50) { nodes { id databaseId author { __typename login avatarUrl } body state submittedAt url } }
    }
    ... on Issue {
      id updatedAt
      comments(last: 50) { nodes { id databaseId author { __typename login avatarUrl } body createdAt url } }
    }
  }
}
"#;
const DISCUSSION_BATCH_SIZE: usize = 10;

/// One top-level comment or review body of a PR/issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscussionComment {
    pub id: String,
    pub database_id: Option<i64>,
    /// `comment` or `review`.
    pub kind: String,
    /// A review's state (`APPROVED`, `CHANGES_REQUESTED`, `COMMENTED`, …).
    pub review_state: Option<String>,
    pub author: Option<gh::PrActor>,
    pub body: String,
    pub created_at: String,
    pub url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDiscussionComment {
    id: String,
    #[serde(default)]
    database_id: Option<i64>,
    #[serde(default)]
    author: Option<gh::PrActor>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    submitted_at: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    url: String,
}

#[derive(Debug, Default, Deserialize)]
struct RawDiscussionConn {
    #[serde(default)]
    nodes: Vec<RawDiscussionComment>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Discussion {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    comments: RawDiscussionConn,
    #[serde(default)]
    reviews: RawDiscussionConn,
}

impl Fetched for Discussion {
    fn supported_id(&self) -> Option<&str> {
        self.id.as_deref()
    }
}

impl Discussion {
    /// Comments and non-empty review bodies, oldest first.
    pub fn into_comments(self) -> Vec<DiscussionComment> {
        let comments = self.comments.nodes.into_iter().map(|c| ("comment", c));
        let reviews = self.reviews.nodes.into_iter().filter(|r| !r.body.trim().is_empty()).map(|r| ("review", r));
        let mut out: Vec<DiscussionComment> = comments
            .chain(reviews)
            .map(|(kind, c)| DiscussionComment {
                id: c.id,
                database_id: c.database_id,
                kind: kind.to_string(),
                review_state: c.state,
                author: c.author,
                body: c.body,
                created_at: c.created_at.or(c.submitted_at).unwrap_or_default(),
                url: c.url,
            })
            .collect();
        out.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        out
    }
}

/// The discussion facet of PRs/issues by node id, in concurrent batches.
pub fn discussions(ids: &[String]) -> HashMap<String, Option<Discussion>> {
    nodes_by_id(DISCUSSION_QUERY, ids, DISCUSSION_BATCH_SIZE)
}

/// Resolve several `owner/name#number` subjects (PRs or issues), concurrently.
/// Failures are left out.
pub fn resolve_many(subjects: Vec<(String, String, i64)>) -> HashMap<(String, String, i64), Node> {
    crate::inbox::concurrent_map(subjects, REFETCH_CONCURRENCY, |(owner, name, number)| {
        let node = resolve(owner, name, *number);
        if let Err(e) = &node {
            crate::logging::warn(&format!("[mirror.resolve_many] {owner}/{name}#{number}: {e}"));
        }
        ((owner.clone(), name.clone(), *number), node.ok())
    })
    .into_iter()
    .filter_map(|(k, n)| Some((k, n?)))
    .collect()
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

    #[test]
    fn discussion_merges_comments_and_review_bodies_oldest_first() {
        let d: Discussion = serde_json::from_value(serde_json::json!({
            "id": "PR_1", "updatedAt": "2026-01-03T00:00:00Z",
            "comments": { "nodes": [
                { "id": "C2", "databaseId": 2, "author": { "__typename": "Bot", "login": "sonar" },
                  "body": "gate", "createdAt": "2026-01-03T00:00:00Z", "url": "u2" }] },
            "reviews": { "nodes": [
                { "id": "R1", "databaseId": 1, "author": { "__typename": "User", "login": "me" },
                  "body": "looks good", "state": "APPROVED", "submittedAt": "2026-01-02T00:00:00Z", "url": "u1" },
                { "id": "R0", "author": null, "body": "  ", "state": "COMMENTED", "submittedAt": "2026-01-01T00:00:00Z", "url": "u0" }] }
        }))
        .unwrap();
        let got = d.into_comments();
        assert_eq!(got.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["R1", "C2"]);
        assert_eq!(got[0].review_state.as_deref(), Some("APPROVED"));
        assert_eq!(got[1].author.as_ref().unwrap().typename.as_deref(), Some("Bot"));
    }

    #[test]
    fn by_head_query_aliases_each_branch_and_uses_only_the_pr_fragment() {
        let q = prs_by_head_query(2);
        assert!(q.contains("$b0: String!, $b1: String!"));
        assert!(q.contains("b1: pullRequests(headRefName: $b1"));
        assert!(q.contains("fragment PrCore"));
        assert!(!q.contains("IssueCore"), "an unused fragment fails the whole query");
        assert!(FRAGMENTS.contains("fragment IssueCore") && FRAGMENTS.contains("fragment PrCore"));
    }

    #[test]
    fn by_head_response_flattens_aliases() {
        let data: ByHeadData = serde_json::from_value(serde_json::json!({
            "repository": { "b0": { "nodes": [] }, "b1": { "nodes": [serde_json::from_str::<serde_json::Value>(PR_NODE_FIXTURE).unwrap()] } }
        }))
        .unwrap();
        let n: usize = data.repository.unwrap().into_values().map(|c| c.nodes.len()).sum();
        assert_eq!(n, 1);
    }

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
    fn probe_deserializes_prs_issues_and_skips_other_types() {
        let data: NodesOf<Probe> = serde_json::from_str(
            r#"{ "nodes": [
              { "__typename": "PullRequest", "id": "PR_1", "updatedAt": "t", "headRefOid": "h",
                "state": "CLOSED", "merged": true, "isDraft": false, "mergeStateStatus": "CLEAN",
                "autoMergeRequest": { "mergeMethod": null },
                "commits": { "nodes": [ { "commit": { "statusCheckRollup": { "state": "FAILURE" } } } ] } },
              { "__typename": "Issue", "id": "I_1", "updatedAt": "t", "state": "OPEN" },
              { "__typename": "Commit" },
              null
            ] }"#,
        )
        .unwrap();
        let ids: Vec<Option<&str>> = data.nodes.iter().map(|n| n.as_ref().and_then(Fetched::supported_id)).collect();
        assert_eq!(ids, vec![Some("PR_1"), Some("I_1"), None, None]);
        let pr = data.nodes[0].as_ref().unwrap();
        assert_eq!(pr.normalized_state().as_deref(), Some("merged"));
        assert_eq!(pr.ci_state().as_deref(), Some("FAILURE"));
        assert_eq!(pr.auto_merge_method().as_deref(), Some("MERGE"));
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
