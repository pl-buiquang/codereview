//! GitHub "inbox" fetch pipeline, ported from the gh-dashboard app. Pure-ish
//! helpers (no DB, no Tauri): they fan out `gh api graphql` / `gh api` searches,
//! merge the results by node id, and shape them into rows for the `items` table.
//! The DB orchestration lives in `commands/inbox.rs`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::json;

use crate::error::AppResult;
use crate::gh;

const PER_SEARCH_LIMIT: i64 = 50;
const TEAM_LIMIT: usize = 20;
const ENRICH_CONCURRENCY: usize = 8;
const SEARCH_CONCURRENCY: usize = 8;
const REFETCH_BATCH_SIZE: usize = 50;
/// Cached viewer (login + teams) is considered fresh for 24h.
pub const VIEWER_TTL_MS: i64 = 24 * 60 * 60 * 1000;

// ---------------------------------------------------------------------------
// GraphQL queries (ported verbatim from gh-dashboard)
// ---------------------------------------------------------------------------

const SEARCH_QUERY: &str = r#"
query Search($q: String!, $first: Int!) {
  search(query: $q, type: ISSUE, first: $first) {
    nodes {
      __typename
      ... on Issue {
        id number title url bodyText state updatedAt
        author { login avatarUrl }
        repository { nameWithOwner }
        comments(last: 1) { nodes { bodyText author { login } } }
      }
      ... on PullRequest {
        id number title url bodyText state isDraft merged updatedAt
        author { login avatarUrl }
        repository { nameWithOwner }
        changedFiles additions deletions reviewDecision
        comments(last: 1) { nodes { bodyText author { login } } }
        commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
      }
    }
  }
}
"#;

const REFETCH_NODES_QUERY: &str = r#"
query RefetchNodes($ids: [ID!]!) {
  nodes(ids: $ids) {
    __typename
    ... on Issue {
      id number title url bodyText state updatedAt
      author { login avatarUrl }
      repository { nameWithOwner }
      comments(last: 1) { nodes { bodyText author { login } } }
    }
    ... on PullRequest {
      id number title url bodyText state isDraft merged updatedAt
      author { login avatarUrl }
      repository { nameWithOwner }
      changedFiles additions deletions reviewDecision
      comments(last: 1) { nodes { bodyText author { login } } }
      commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
    }
  }
}
"#;

const VIEWER_QUERY: &str = r#"
query Viewer {
  viewer {
    login
    organizations(first: 50) { nodes { login } }
  }
}
"#;

const ORG_TEAMS_QUERY: &str = r#"
query OrgTeams($org: String!, $login: String!, $cursor: String) {
  organization(login: $org) {
    teams(first: 100, userLogins: [$login], after: $cursor) {
      pageInfo { hasNextPage endCursor }
      nodes { slug }
    }
  }
}
"#;

// ---------------------------------------------------------------------------
// GraphQL response shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchNode {
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
    pub author: Option<Author>,
    pub repository: RepoRef,
    #[serde(default)]
    pub changed_files: Option<i64>,
    #[serde(default)]
    pub additions: Option<i64>,
    #[serde(default)]
    pub deletions: Option<i64>,
    #[serde(default)]
    pub review_decision: Option<String>,
    #[serde(default)]
    pub comments: CommentConn,
    #[serde(default)]
    pub commits: Option<CommitConn>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Author {
    pub login: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoRef {
    pub name_with_owner: String,
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
    pub author: Option<AuthorLogin>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthorLogin {
    pub login: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommitConn {
    #[serde(default)]
    pub nodes: Vec<CommitNode>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommitNode {
    pub commit: CommitInner,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitInner {
    #[serde(default)]
    pub status_check_rollup: Option<StatusRollup>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StatusRollup {
    pub state: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchData {
    search: SearchConn,
}

#[derive(Debug, Deserialize)]
struct SearchConn {
    nodes: Vec<SearchNode>,
}

#[derive(Debug, Deserialize)]
struct RefetchData {
    nodes: Vec<Option<SearchNode>>,
}

#[derive(Debug, Deserialize)]
struct ViewerData {
    viewer: Viewer,
}

#[derive(Debug, Deserialize)]
struct Viewer {
    login: String,
    organizations: OrgConn,
}

#[derive(Debug, Deserialize)]
struct OrgConn {
    nodes: Vec<OrgNode>,
}

#[derive(Debug, Deserialize)]
struct OrgNode {
    login: String,
}

#[derive(Debug, Deserialize)]
struct OrgTeamsData {
    organization: Option<OrgTeams>,
}

#[derive(Debug, Deserialize)]
struct OrgTeams {
    teams: Option<TeamConn>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TeamConn {
    page_info: PageInfo,
    nodes: Vec<TeamNode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TeamNode {
    slug: String,
}

// ---------------------------------------------------------------------------
// Working types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Reason {
    pub reason: String,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct ReasonedNode {
    pub node: SearchNode,
    pub reasons: Vec<Reason>,
}

#[derive(Debug, Clone)]
pub struct ViewerInfo {
    pub login: String,
    pub team_slugs: Vec<String>,
}

/// Flattened row shape written to the `items` table (the upsert binds these).
#[derive(Debug, Clone)]
pub struct ItemInput {
    pub id: String,
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
}

/// A PR queued for top-changed-files enrichment.
pub struct EnrichTarget {
    pub id: String,
    pub repo: String,
    pub number: i64,
}

// ---------------------------------------------------------------------------
// Concurrency helper
// ---------------------------------------------------------------------------

/// Run `f` over `items` with at most `limit` workers, preserving input order.
/// Each `gh` call is an independent subprocess, so OS threads are the natural fit
/// (and avoid pulling in an async runtime — see CLAUDE.md).
fn concurrent_map<T, R, F>(items: Vec<T>, limit: usize, f: F) -> Vec<R>
where
    T: Send + Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    let n = items.len();
    if n == 0 {
        return Vec::new();
    }
    let slots: Vec<Mutex<Option<R>>> = (0..n).map(|_| Mutex::new(None)).collect();
    let cursor = AtomicUsize::new(0);
    let workers = limit.clamp(1, n);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let i = cursor.fetch_add(1, Ordering::Relaxed);
                if i >= n {
                    break;
                }
                let r = f(&items[i]);
                *slots[i].lock().unwrap() = Some(r);
            });
        }
    });
    slots
        .into_iter()
        .map(|m| m.into_inner().unwrap().expect("worker filled every slot"))
        .collect()
}

// ---------------------------------------------------------------------------
// Pipeline steps
// ---------------------------------------------------------------------------

/// Collapse internal whitespace and truncate to `max` chars (matching the TS
/// `snippet`). Returns `None` for empty/missing input.
pub fn snippet(s: Option<&str>, max: usize) -> Option<String> {
    let s = s?;
    if s.is_empty() {
        return None;
    }
    let collapsed: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    let chars: Vec<char> = collapsed.chars().collect();
    if chars.len() > max {
        let head: String = chars[..max.saturating_sub(1)].iter().collect();
        Some(format!("{head}…"))
    } else {
        Some(collapsed)
    }
}

/// Project a GraphQL node into the flat `items` row shape.
pub fn node_to_input(node: &SearchNode, top_files_json: Option<String>) -> ItemInput {
    let is_pr = node.typename == "PullRequest";
    let typ = if is_pr { "pr" } else { "issue" };
    let last_comment = node.comments.nodes.first();
    let state = if is_pr && node.merged {
        Some("merged".to_string())
    } else {
        node.state.as_ref().map(|s| s.to_lowercase())
    };
    let ci_state = if is_pr {
        node.commits
            .as_ref()
            .and_then(|c| c.nodes.first())
            .and_then(|n| n.commit.status_check_rollup.as_ref())
            .and_then(|r| r.state.as_ref())
            .map(|s| s.to_lowercase())
    } else {
        None
    };
    ItemInput {
        id: node.id.clone(),
        typ: typ.to_string(),
        number: node.number,
        repo: node.repository.name_with_owner.clone(),
        title: node.title.clone(),
        url: node.url.clone(),
        author_login: node.author.as_ref().and_then(|a| a.login.clone()),
        author_avatar: node.author.as_ref().and_then(|a| a.avatar_url.clone()),
        state,
        is_draft: node.is_draft,
        body: snippet(node.body_text.as_deref(), 280),
        latest_comment: snippet(last_comment.and_then(|c| c.body_text.as_deref()), 280),
        latest_actor: last_comment
            .and_then(|c| c.author.as_ref())
            .and_then(|a| a.login.clone()),
        updated_at: node.updated_at.clone(),
        files_changed: node.changed_files,
        additions: node.additions,
        deletions: node.deletions,
        top_files_json,
        ci_state,
        review_decision: node.review_decision.clone(),
    }
}

/// Merge per-search buckets by node id, unioning reasons and keeping the most
/// recently updated copy of each node. Output order follows first appearance.
pub fn merge_reasons(buckets: Vec<Vec<ReasonedNode>>) -> Vec<ReasonedNode> {
    let mut order: Vec<String> = Vec::new();
    let mut map: HashMap<String, ReasonedNode> = HashMap::new();
    for bucket in buckets {
        for r in bucket {
            match map.get_mut(&r.node.id) {
                None => {
                    order.push(r.node.id.clone());
                    map.insert(r.node.id.clone(), r);
                }
                Some(existing) => {
                    for reason in r.reasons {
                        let dup = existing
                            .reasons
                            .iter()
                            .any(|e| e.reason == reason.reason && e.detail == reason.detail);
                        if !dup {
                            existing.reasons.push(reason);
                        }
                    }
                    // ISO8601 timestamps compare correctly lexicographically.
                    if r.node.updated_at > existing.node.updated_at {
                        existing.node = r.node;
                    }
                }
            }
        }
    }
    order.into_iter().filter_map(|id| map.remove(&id)).collect()
}

fn scoped(scope: Option<&str>, q: &str) -> String {
    match scope {
        Some(s) if !s.trim().is_empty() => format!("{} {}", s.trim(), q),
        _ => q.to_string(),
    }
}

struct SearchTask {
    query: String,
    reason: &'static str,
    detail: String,
}

fn build_search_tasks(viewer: &ViewerInfo, scope: Option<&str>) -> Vec<SearchTask> {
    let mut tasks = vec![
        ("is:issue is:open assignee:@me", "assigned"),
        ("is:pr is:open assignee:@me", "assigned"),
        ("is:pr is:open user-review-requested:@me", "direct_review"),
        ("is:issue is:open mentions:@me", "mention"),
        ("is:pr is:open mentions:@me", "mention"),
        ("is:issue is:open author:@me", "author"),
        ("is:pr is:open author:@me", "author"),
    ]
    .into_iter()
    .map(|(q, reason)| SearchTask {
        query: scoped(scope, q),
        reason,
        detail: String::new(),
    })
    .collect::<Vec<_>>();

    for team in viewer.team_slugs.iter().take(TEAM_LIMIT) {
        tasks.push(SearchTask {
            query: scoped(scope, &format!("is:pr is:open team-review-requested:{team}")),
            reason: "team_review",
            detail: team.clone(),
        });
    }
    tasks
}

fn run_search(task: &SearchTask) -> Vec<ReasonedNode> {
    let vars = json!({ "q": task.query, "first": PER_SEARCH_LIMIT });
    match gh::graphql::<SearchData>(SEARCH_QUERY, vars) {
        Ok(data) => data
            .search
            .nodes
            .into_iter()
            .filter(|n| n.typename == "Issue" || n.typename == "PullRequest")
            .map(|node| ReasonedNode {
                node,
                reasons: vec![Reason {
                    reason: task.reason.to_string(),
                    detail: task.detail.clone(),
                }],
            })
            .collect(),
        Err(e) => {
            eprintln!("[inbox.search] q={:?} failed: {e}", task.query);
            Vec::new()
        }
    }
}

/// Fan out all attention searches concurrently and merge them.
pub fn run_all_searches(viewer: &ViewerInfo, scope: Option<&str>) -> Vec<ReasonedNode> {
    let tasks = build_search_tasks(viewer, scope);
    let buckets = concurrent_map(tasks, SEARCH_CONCURRENCY, run_search);
    merge_reasons(buckets)
}

/// Fetch the viewer's login + team slugs (`org/team`) via GraphQL, paginating the
/// per-org team list. Per-org failures (e.g. no `read:org` access) are skipped.
pub fn fetch_viewer() -> AppResult<ViewerInfo> {
    let data: ViewerData = gh::graphql(VIEWER_QUERY, json!({}))?;
    let login = data.viewer.login;
    let orgs: Vec<String> = data.viewer.organizations.nodes.into_iter().map(|n| n.login).collect();

    let mut team_slugs = Vec::new();
    for org in orgs {
        let mut cursor: Option<String> = None;
        loop {
            let vars = json!({ "org": org, "login": login, "cursor": cursor });
            let teams = match gh::graphql::<OrgTeamsData>(ORG_TEAMS_QUERY, vars) {
                Ok(d) => d.organization.and_then(|o| o.teams),
                Err(e) => {
                    eprintln!("[inbox.viewer] org={org} teams failed: {e}");
                    break;
                }
            };
            let Some(teams) = teams else { break };
            for t in teams.nodes {
                team_slugs.push(format!("{org}/{}", t.slug));
            }
            if teams.page_info.has_next_page {
                match teams.page_info.end_cursor {
                    Some(c) => cursor = Some(c),
                    None => break,
                }
            } else {
                break;
            }
        }
    }
    Ok(ViewerInfo { login, team_slugs })
}

/// Top-5-by-change changed files for a PR, as a JSON array string. `None` on any
/// failure (degrades gracefully under rate limits rather than aborting a refresh).
pub fn enrich_top_files(repo: &str, number: i64) -> Option<String> {
    let (owner, name) = repo.split_once('/')?;
    match gh::pr_files(owner, name, number) {
        Ok(mut files) => {
            files.sort_by(|a, b| b.changes.cmp(&a.changes));
            files.truncate(5);
            let top: Vec<_> = files
                .iter()
                .map(|f| {
                    json!({
                        "path": f.filename,
                        "additions": f.additions,
                        "deletions": f.deletions,
                        "changes": f.changes,
                    })
                })
                .collect();
            Some(serde_json::Value::Array(top).to_string())
        }
        Err(e) => {
            eprintln!("[inbox.enrich] top-files failed for {repo}#{number}: {e}");
            None
        }
    }
}

/// Enrich many PRs concurrently. Returns `(node_id, top_files_json?)` per input.
pub fn enrich_all(prs: Vec<EnrichTarget>) -> Vec<(String, Option<String>)> {
    concurrent_map(prs, ENRICH_CONCURRENCY, |p| {
        (p.id.clone(), enrich_top_files(&p.repo, p.number))
    })
}

/// Refetch nodes by id (used to detect items that have closed/merged since they
/// were last in the inbox). Batched; missing/inaccessible ids simply don't appear.
pub fn refetch_nodes(ids: &[String]) -> HashMap<String, SearchNode> {
    let mut result = HashMap::new();
    for chunk in ids.chunks(REFETCH_BATCH_SIZE) {
        let vars = json!({ "ids": chunk });
        match gh::graphql::<RefetchData>(REFETCH_NODES_QUERY, vars) {
            Ok(data) => {
                for node in data.nodes.into_iter().flatten() {
                    if node.typename == "Issue" || node.typename == "PullRequest" {
                        result.insert(node.id.clone(), node);
                    }
                }
            }
            Err(e) => eprintln!("[inbox.refetch] batch failed: {e}"),
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, typename: &str, updated_at: &str) -> SearchNode {
        SearchNode {
            typename: typename.to_string(),
            id: id.to_string(),
            number: 1,
            title: "t".into(),
            url: "u".into(),
            body_text: None,
            state: Some("OPEN".into()),
            is_draft: false,
            merged: false,
            updated_at: updated_at.to_string(),
            author: None,
            repository: RepoRef {
                name_with_owner: "acme/widget".into(),
            },
            changed_files: None,
            additions: None,
            deletions: None,
            review_decision: None,
            comments: CommentConn::default(),
            commits: None,
        }
    }

    fn reasoned(id: &str, typename: &str, updated_at: &str, reason: &str) -> ReasonedNode {
        ReasonedNode {
            node: node(id, typename, updated_at),
            reasons: vec![Reason {
                reason: reason.to_string(),
                detail: String::new(),
            }],
        }
    }

    #[test]
    fn snippet_collapses_and_truncates() {
        assert_eq!(snippet(None, 10), None);
        assert_eq!(snippet(Some(""), 10), None);
        assert_eq!(snippet(Some("  \n\t  "), 10), None);
        assert_eq!(snippet(Some("a   b\n c"), 10).as_deref(), Some("a b c"));
        // Truncation keeps max-1 chars + ellipsis.
        assert_eq!(snippet(Some("abcdefghij"), 5).as_deref(), Some("abcd…"));
    }

    #[test]
    fn merge_reasons_unions_and_keeps_latest_node() {
        let buckets = vec![
            vec![reasoned("X", "PullRequest", "2024-01-01T00:00:00Z", "author")],
            vec![
                reasoned("X", "PullRequest", "2024-02-01T00:00:00Z", "mention"),
                reasoned("Y", "Issue", "2024-01-15T00:00:00Z", "assigned"),
            ],
            // Duplicate reason on X must not be added twice.
            vec![reasoned("X", "PullRequest", "2024-01-10T00:00:00Z", "author")],
        ];
        let merged = merge_reasons(buckets);
        assert_eq!(merged.len(), 2);
        let x = &merged[0];
        assert_eq!(x.node.id, "X");
        // Newest updatedAt wins.
        assert_eq!(x.node.updated_at, "2024-02-01T00:00:00Z");
        let mut reasons: Vec<&str> = x.reasons.iter().map(|r| r.reason.as_str()).collect();
        reasons.sort_unstable();
        assert_eq!(reasons, vec!["author", "mention"]);
        assert_eq!(merged[1].node.id, "Y");
    }

    #[test]
    fn node_to_input_maps_pr_fields() {
        let mut n = node("X", "PullRequest", "2024-02-01T00:00:00Z");
        n.merged = true;
        n.changed_files = Some(3);
        n.commits = Some(CommitConn {
            nodes: vec![CommitNode {
                commit: CommitInner {
                    status_check_rollup: Some(StatusRollup {
                        state: Some("SUCCESS".into()),
                    }),
                },
            }],
        });
        let input = node_to_input(&n, Some("[]".into()));
        assert_eq!(input.typ, "pr");
        assert_eq!(input.state.as_deref(), Some("merged"));
        assert_eq!(input.ci_state.as_deref(), Some("success"));
        assert_eq!(input.files_changed, Some(3));
        assert_eq!(input.top_files_json.as_deref(), Some("[]"));
    }

    #[test]
    fn node_to_input_issue_has_no_ci_or_merge() {
        let n = node("Y", "Issue", "2024-02-01T00:00:00Z");
        let input = node_to_input(&n, None);
        assert_eq!(input.typ, "issue");
        assert_eq!(input.state.as_deref(), Some("open"));
        assert_eq!(input.ci_state, None);
    }
}
