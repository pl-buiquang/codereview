//! GitHub "inbox" discovery pipeline, ported from the gh-dashboard app. Pure-ish
//! helpers (no DB, no Tauri): they fan out the attention searches, merge the
//! results by node id. Results land in the mirror;
//! the DB orchestration lives in `commands/inbox.rs`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::json;

use crate::error::AppResult;
use crate::gh;
use crate::mirror::{self, Node};

const PER_SEARCH_LIMIT: i64 = 50;
const TEAM_LIMIT: usize = 20;
const SEARCH_CONCURRENCY: usize = 8;
/// Cached viewer (login + teams) is considered fresh for 24h.
pub const VIEWER_TTL_MS: i64 = 24 * 60 * 60 * 1000;

// ---------------------------------------------------------------------------
// GraphQL queries (viewer + teams; searches use the mirror fragment)
// ---------------------------------------------------------------------------

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
    pub node: Node,
    pub reasons: Vec<Reason>,
}

#[derive(Debug, Clone)]
pub struct ViewerInfo {
    pub login: String,
    pub team_slugs: Vec<String>,
}

// ---------------------------------------------------------------------------
// Concurrency helper
// ---------------------------------------------------------------------------

/// Run `f` over `items` with at most `limit` workers, preserving input order.
/// Each `gh` call is an independent subprocess, so OS threads are the natural fit
/// (and avoid pulling in an async runtime — see CLAUDE.md).
pub(crate) fn concurrent_map<T, R, F>(items: Vec<T>, limit: usize, f: F) -> Vec<R>
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

/// `None` when the search failed, so the caller knows the results are partial.
fn run_search(task: &SearchTask) -> Option<Vec<ReasonedNode>> {
    match mirror::fetch::search(&task.query, PER_SEARCH_LIMIT) {
        Ok(nodes) => Some(
            nodes
                .into_iter()
                .map(|node| ReasonedNode {
                    node,
                    reasons: vec![Reason {
                        reason: task.reason.to_string(),
                        detail: task.detail.clone(),
                    }],
                })
                .collect(),
        ),
        Err(e) => {
            crate::logging::warn(&format!("[inbox.search] q={:?} failed: {e}", task.query));
            None
        }
    }
}

/// Fan out all attention searches concurrently and merge them. The flag is
/// true when every search succeeded: only then does a missing reason mean the
/// item no longer matches.
pub fn run_all_searches(viewer: &ViewerInfo, scope: Option<&str>) -> (Vec<ReasonedNode>, bool) {
    let tasks = build_search_tasks(viewer, scope);
    let results = concurrent_map(tasks, SEARCH_CONCURRENCY, run_search);
    let complete = results.iter().all(Option::is_some);
    (merge_reasons(results.into_iter().flatten().collect()), complete)
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
                    crate::logging::warn(&format!("[inbox.viewer] org={org} teams failed: {e}"));
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, typename: &str, updated_at: &str) -> Node {
        serde_json::from_value(json!({
            "__typename": typename, "id": id, "number": 1, "title": "t", "url": "u",
            "state": "OPEN", "updatedAt": updated_at,
            "repository": { "nameWithOwner": "acme/widget" }
        }))
        .unwrap()
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
}
