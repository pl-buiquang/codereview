//! DB side of the mirror: upserts from fetched nodes and facets, readers that
//! rebuild the frontend DTOs, and GC. Callers hold the DB lock; nothing here
//! touches the network.

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};

use super::fetch::{Actor, Node};
use crate::board::{self, ThreadMode};
use crate::error::AppResult;
use crate::gh::{PrActor, PrAutoMerge, PrDetail, PrMeta, PrThread};

/// Unreferenced mirror rows are kept this long after their last sync.
const GC_AFTER_DAYS: i64 = 30;

/// Collapse internal whitespace and truncate to `max` chars. `None` for
/// empty/missing input.
pub fn snippet(s: Option<&str>, max: usize) -> Option<String> {
    let collapsed: String = s?.split_whitespace().collect::<Vec<_>>().join(" ");
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

/// The repository row for a GitHub `owner/name`, auto-adding a hidden
/// (unregistered) one when the app doesn't know the repo yet.
pub fn ensure_repo(conn: &Connection, owner: &str, name: &str, ts: &str) -> AppResult<i64> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM repository
             WHERE remote_owner = ?1 COLLATE NOCASE AND remote_name = ?2 COLLATE NOCASE
             ORDER BY id LIMIT 1",
            params![owner, name],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO repository (local_path, remote_owner, remote_name, added_at, registered_at)
         VALUES (NULL, ?1, ?2, ?3, NULL)",
        params![owner, name, ts],
    )?;
    Ok(conn.last_insert_rowid())
}

fn upsert_user(conn: &Connection, actor: Option<&Actor>, ts: &str) -> AppResult<()> {
    let Some(login) = actor.and_then(|a| a.login.as_deref()) else {
        return Ok(());
    };
    conn.execute(
        "INSERT INTO gh_user (login, avatar_url, fetched_at, is_bot) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(login) DO UPDATE SET
            avatar_url = COALESCE(excluded.avatar_url, gh_user.avatar_url),
            fetched_at = excluded.fetched_at,
            -- Only the author selection carries the type; other actors keep what's known.
            is_bot = MAX(gh_user.is_bot, excluded.is_bot)",
        params![login, actor.and_then(|a| a.avatar_url.as_deref()), ts, actor.is_some_and(Actor::is_bot)],
    )?;
    Ok(())
}

/// Upsert a fetched Issue/PR (core + status facets for a PR).
pub fn upsert_node(conn: &Connection, node: &Node, ts: &str) -> AppResult<()> {
    let Some((owner, name)) = node.owner_name() else {
        return Ok(());
    };
    let repo_id = ensure_repo(conn, owner, name, ts)?;
    upsert_user(conn, node.author.as_ref(), ts)?;
    let body = snippet(node.body_text.as_deref(), 280);
    let last = node.latest_comment();
    let latest_comment = snippet(last.and_then(|c| c.body_text.as_deref()), 280);
    let latest_actor = last.and_then(|c| c.author.as_ref()).and_then(|a| a.login.clone());
    let table = if node.is_pr() { "gh_pr" } else { "gh_issue" };
    // A number can only belong to one node per repo; a stale row (e.g. a
    // recreated repo) would otherwise fail the UNIQUE constraint.
    conn.execute(
        &format!("DELETE FROM {table} WHERE repo_id = ?1 AND number = ?2 AND node_id != ?3"),
        params![repo_id, node.number, node.id],
    )?;

    if !node.is_pr() {
        conn.execute(
            "INSERT INTO gh_issue (node_id, repo_id, number, title, url, author_login, state,
                body_snippet, latest_comment, latest_actor, gh_updated_at, closed_at, fetched_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
             ON CONFLICT(node_id) DO UPDATE SET
                repo_id = excluded.repo_id, number = excluded.number, title = excluded.title,
                url = excluded.url, author_login = excluded.author_login, state = excluded.state,
                body_snippet = excluded.body_snippet, latest_comment = excluded.latest_comment,
                latest_actor = excluded.latest_actor, gh_updated_at = excluded.gh_updated_at,
                closed_at = excluded.closed_at, fetched_at = excluded.fetched_at",
            params![
                node.id,
                repo_id,
                node.number,
                node.title,
                node.url,
                node.author_login(),
                node.normalized_state(),
                body,
                latest_comment,
                latest_actor,
                node.updated_at,
                node.closed_at,
                ts,
            ],
        )?;
        return Ok(());
    }

    let closing_ids = node
        .closing_issues_references
        .as_ref()
        .map(|c| serde_json::to_string(&c.nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>()))
        .transpose()?;
    conn.execute(
        "INSERT INTO gh_pr (node_id, repo_id, number, title, url, author_login, state, is_draft,
            body_snippet, base_ref, head_ref, head_sha, review_decision, review_count,
            additions, deletions, changed_files, latest_comment, latest_actor,
            gh_updated_at, closed_at, fetched_at, closing_issue_ids)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)
         ON CONFLICT(node_id) DO UPDATE SET
            closing_issue_ids = COALESCE(excluded.closing_issue_ids, gh_pr.closing_issue_ids),
            repo_id = excluded.repo_id, number = excluded.number, title = excluded.title,
            url = excluded.url, author_login = excluded.author_login, state = excluded.state,
            is_draft = excluded.is_draft, body_snippet = excluded.body_snippet,
            base_ref = excluded.base_ref, head_ref = excluded.head_ref, head_sha = excluded.head_sha,
            review_decision = excluded.review_decision, review_count = excluded.review_count,
            additions = excluded.additions, deletions = excluded.deletions,
            changed_files = excluded.changed_files, latest_comment = excluded.latest_comment,
            latest_actor = excluded.latest_actor, gh_updated_at = excluded.gh_updated_at,
            closed_at = excluded.closed_at, fetched_at = excluded.fetched_at",
        params![
            node.id,
            repo_id,
            node.number,
            node.title,
            node.url,
            node.author_login(),
            node.normalized_state(),
            node.is_draft as i64,
            body,
            node.base_ref_name,
            node.head_ref_name,
            node.head_ref_oid,
            node.review_decision,
            node.reviews.as_ref().map(|r| r.total_count),
            node.additions,
            node.deletions,
            node.changed_files,
            latest_comment,
            latest_actor,
            node.updated_at,
            node.closed_at,
            ts,
            closing_ids,
        ],
    )?;
    let auto_merge_by = node
        .auto_merge_request
        .as_ref()
        .and_then(|a| a.enabled_by.as_ref())
        .and_then(|a| a.login.clone());
    // The detail-only columns describe one head; a new head drops them.
    conn.execute(
        "INSERT INTO gh_pr_status (pr_id, head_sha, ci_state, mergeable, merge_state_status,
            viewer_can_update_branch, auto_merge_method, auto_merge_by, stack_number, stack_position,
            fetched_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
         ON CONFLICT(pr_id) DO UPDATE SET
            checks_json = CASE WHEN gh_pr_status.head_sha IS excluded.head_sha
                               THEN gh_pr_status.checks_json END,
            head_sha = excluded.head_sha, ci_state = excluded.ci_state,
            mergeable = excluded.mergeable, merge_state_status = excluded.merge_state_status,
            viewer_can_update_branch = excluded.viewer_can_update_branch,
            auto_merge_method = excluded.auto_merge_method, auto_merge_by = excluded.auto_merge_by,
            stack_number = excluded.stack_number, stack_position = excluded.stack_position,
            fetched_at = excluded.fetched_at",
        params![
            node.id,
            node.head_ref_oid,
            node.ci_state(),
            node.mergeable,
            node.merge_state_status,
            node.viewer_can_update_branch as i64,
            node.auto_merge_method(),
            auto_merge_by,
            node.stack.as_ref().map(|s| s.number),
            node.stack_entry.as_ref().map(|s| s.position),
            ts,
        ],
    )?;
    Ok(())
}

/// Upsert the PR-header fetch: the node plus body, labels, reviews and checks.
pub fn upsert_pr_detail(conn: &Connection, detail: &PrDetail, ts: &str) -> AppResult<()> {
    upsert_node(conn, &detail.node, ts)?;
    let m = &detail.meta;
    conn.execute("UPDATE gh_pr SET body = ?1 WHERE node_id = ?2", params![m.body, detail.node.id])?;
    for r in &m.reviews {
        upsert_user(conn, r.author.as_ref().map(actor_of).as_ref(), ts)?;
    }
    conn.execute(
        "UPDATE gh_pr_status SET checks_json = ?1, labels_json = ?2, latest_reviews_json = ?3
         WHERE pr_id = ?4",
        params![
            serde_json::to_string(&m.checks)?,
            serde_json::to_string(&m.labels)?,
            serde_json::to_string(&m.reviews)?,
            detail.node.id,
        ],
    )?;
    Ok(())
}

fn actor_of(a: &PrActor) -> Actor {
    Actor { type_name: None, login: a.login.clone(), avatar_url: a.avatar_url.clone() }
}

/// The viewer login cached by the inbox (`meta.viewer_login`).
pub fn viewer_login(conn: &Connection) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = 'viewer_login'", [], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
}

/// Store a PR's review threads, keyed by the PR's mirrored `updatedAt`, with
/// the viewer's pending-threads count (mine: last word isn't mine; others':
/// threads I started that someone answered).
pub fn upsert_threads(conn: &Connection, pr_id: &str, threads: &[PrThread], ts: &str) -> AppResult<()> {
    let pr: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT gh_updated_at, author_login FROM gh_pr WHERE node_id = ?1",
            params![pr_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((updated_at, author)) = pr else {
        return Ok(());
    };
    let pending = viewer_login(conn).map(|viewer| {
        let mode = if author.as_deref() == Some(viewer.as_str()) { ThreadMode::Mine } else { ThreadMode::Reviewing };
        board::pending_threads(threads, &viewer, mode) as i64
    });
    conn.execute(
        "INSERT INTO gh_pr_threads (pr_id, gh_updated_at, pending_count, threads_json, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(pr_id) DO UPDATE SET
            gh_updated_at = excluded.gh_updated_at, pending_count = excluded.pending_count,
            threads_json = excluded.threads_json, fetched_at = excluded.fetched_at",
        params![pr_id, updated_at, pending, serde_json::to_string(threads)?, ts],
    )?;
    Ok(())
}

/// The mirrored node id of `owner/name#number`, if the mirror has it.
pub fn pr_node_id(conn: &Connection, owner: &str, name: &str, number: i64) -> AppResult<Option<String>> {
    conn.query_row(
        "SELECT p.node_id FROM gh_pr p JOIN repository r ON r.id = p.repo_id
         WHERE r.remote_owner = ?1 COLLATE NOCASE AND r.remote_name = ?2 COLLATE NOCASE
           AND p.number = ?3",
        params![owner, name, number],
        |r| r.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn parse_json<T: serde::de::DeserializeOwned>(s: Option<String>) -> Vec<T> {
    s.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// The review header's PR metadata as last mirrored, if the mirror has the PR.
pub fn read_pr_meta(conn: &Connection, owner: &str, name: &str, number: i64) -> AppResult<Option<PrMeta>> {
    let Some(id) = pr_node_id(conn, owner, name, number)? else {
        return Ok(None);
    };
    conn.query_row(
        "SELECT p.number, p.title, p.url, p.body, p.state, p.is_draft, p.head_sha,
                s.mergeable, s.merge_state_status, s.viewer_can_update_branch, p.review_decision,
                s.auto_merge_method, s.auto_merge_by, p.additions, p.deletions, p.changed_files,
                p.author_login, u.avatar_url, s.labels_json, s.latest_reviews_json, s.ci_state,
                s.checks_json
         FROM gh_pr p
         LEFT JOIN gh_pr_status s ON s.pr_id = p.node_id
         LEFT JOIN gh_user u ON u.login = p.author_login
         WHERE p.node_id = ?1",
        params![id],
        |r| {
            let auto_merge_method: Option<String> = r.get(11)?;
            let auto_merge_by: Option<String> = r.get(12)?;
            let author_login: Option<String> = r.get(16)?;
            Ok(PrMeta {
                number: r.get(0)?,
                title: r.get(1)?,
                url: r.get(2)?,
                body: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                state: r.get::<_, Option<String>>(4)?.unwrap_or_default().to_uppercase(),
                is_draft: r.get::<_, i64>(5)? != 0,
                head_ref_oid: r.get(6)?,
                mergeable: r.get(7)?,
                merge_state_status: r.get(8)?,
                viewer_can_update_branch: r.get::<_, Option<i64>>(9)?.unwrap_or(0) != 0,
                review_decision: r.get(10)?,
                auto_merge: auto_merge_method.map(|m| PrAutoMerge {
                    merge_method: Some(m),
                    enabled_by: auto_merge_by.map(|login| PrActor { login: Some(login), avatar_url: None, typename: None }),
                }),
                additions: r.get::<_, Option<i64>>(13)?.unwrap_or(0),
                deletions: r.get::<_, Option<i64>>(14)?.unwrap_or(0),
                changed_files: r.get::<_, Option<i64>>(15)?.unwrap_or(0),
                author: author_login.map(|login| PrActor { login: Some(login), avatar_url: r.get(17).ok().flatten(), typename: None }),
                labels: parse_json(r.get(18)?),
                reviews: parse_json(r.get(19)?),
                ci_state: r.get(20)?,
                checks: parse_json(r.get(21)?),
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

/// A PR's review threads as last mirrored, if they were ever fetched.
pub fn read_threads(conn: &Connection, owner: &str, name: &str, number: i64) -> AppResult<Option<Vec<PrThread>>> {
    let Some(id) = pr_node_id(conn, owner, name, number)? else {
        return Ok(None);
    };
    let json: Option<String> = conn
        .query_row("SELECT threads_json FROM gh_pr_threads WHERE pr_id = ?1", params![id], |r| r.get(0))
        .optional()?
        .flatten();
    Ok(json.and_then(|s| serde_json::from_str(&s).ok()))
}

/// Wipe the mirror (facets cascade). User tables keep their soft refs and show
/// placeholders until the next sync refetches them.
/// Store a subject's discussion facet (top-level comments + review bodies).
pub fn upsert_discussion(
    conn: &Connection,
    subject_id: &str,
    gh_updated_at: Option<&str>,
    comments: &[crate::mirror::fetch::DiscussionComment],
    ts: &str,
) -> AppResult<()> {
    conn.execute(
        "INSERT INTO gh_discussion (subject_id, gh_updated_at, comments_json, fetched_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(subject_id) DO UPDATE SET gh_updated_at = excluded.gh_updated_at,
           comments_json = excluded.comments_json, fetched_at = excluded.fetched_at",
        params![subject_id, gh_updated_at, serde_json::to_string(comments).unwrap_or_else(|_| "[]".into()), ts],
    )?;
    Ok(())
}

/// Store the notified PRs/issues; `subject_node_id` is filled in by the caller
/// once the subject is resolved into the mirror.
pub fn upsert_notification(conn: &Connection, n: &crate::gh::Notification, subject_node_id: Option<&str>, ts: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO gh_notification (id, subject_type, subject_node_id, repo, number, title, reason, unread, updated_at, last_read_at, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(id) DO UPDATE SET subject_type = excluded.subject_type,
           subject_node_id = COALESCE(excluded.subject_node_id, subject_node_id),
           repo = excluded.repo, number = excluded.number, title = excluded.title, reason = excluded.reason,
           unread = excluded.unread, updated_at = excluded.updated_at, last_read_at = excluded.last_read_at,
           fetched_at = excluded.fetched_at",
        params![
            n.id, n.subject_type, subject_node_id, format!("{}/{}", n.owner, n.name), n.number, n.title,
            n.reason, n.unread, n.updated_at, n.last_read_at, ts
        ],
    )?;
    Ok(())
}

pub fn reset(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        "DELETE FROM gh_pr; DELETE FROM gh_issue; DELETE FROM gh_user;
         DELETE FROM gh_notification; DELETE FROM gh_discussion;",
    )?;
    Ok(())
}

/// Delete mirror rows nothing references that haven't synced in 30 days, then
/// auto-added repositories nothing refers to any more. Returns rows deleted.
pub fn gc(conn: &Connection) -> AppResult<usize> {
    let cutoff = (Utc::now() - chrono::Duration::days(GC_AFTER_DAYS)).to_rfc3339();
    let mut n = conn.execute(
        "DELETE FROM gh_pr WHERE fetched_at < ?1
           AND node_id NOT IN (SELECT id FROM inbox_entry)
           AND node_id NOT IN (SELECT id FROM board_card)
           AND node_id NOT IN (SELECT gh_pr_id FROM target WHERE gh_pr_id IS NOT NULL)
           AND node_id NOT IN (SELECT subject_node_id FROM gh_notification WHERE subject_node_id IS NOT NULL)",
        params![cutoff],
    )?;
    n += conn.execute(
        "DELETE FROM gh_issue WHERE fetched_at < ?1
           AND node_id NOT IN (SELECT id FROM inbox_entry)
           AND node_id NOT IN (SELECT id FROM board_card)
           AND node_id NOT IN (SELECT subject_node_id FROM gh_notification WHERE subject_node_id IS NOT NULL)",
        params![cutoff],
    )?;
    n += conn.execute("DELETE FROM gh_notification WHERE updated_at < ?1", params![cutoff])?;
    n += conn.execute(
        "DELETE FROM gh_discussion
         WHERE subject_id NOT IN (SELECT node_id FROM gh_pr) AND subject_id NOT IN (SELECT node_id FROM gh_issue)",
        [],
    )?;
    n += conn.execute(
        "DELETE FROM gh_user WHERE fetched_at < ?1
           AND login NOT IN (SELECT author_login FROM gh_pr WHERE author_login IS NOT NULL)
           AND login NOT IN (SELECT author_login FROM gh_issue WHERE author_login IS NOT NULL)",
        params![cutoff],
    )?;
    n += conn.execute(
        "DELETE FROM repository WHERE registered_at IS NULL AND local_path IS NULL
           AND id NOT IN (SELECT repo_id FROM target)
           AND id NOT IN (SELECT repo_id FROM gh_pr)
           AND id NOT IN (SELECT repo_id FROM gh_issue)",
        [],
    )?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;

    fn pr_node(id: &str, repo: &str, number: i64) -> Node {
        serde_json::from_value(serde_json::json!({
            "__typename": "PullRequest", "id": id, "number": number, "title": "t", "url": "u",
            "bodyText": "  a\n b ", "state": "OPEN", "updatedAt": "2024-01-01T00:00:00Z",
            "headRefName": "feat", "baseRefName": "main", "headRefOid": "h1",
            "author": { "login": "octo", "avatarUrl": "https://a/o.png" },
            "repository": { "nameWithOwner": repo },
            "commits": { "nodes": [ { "commit": { "statusCheckRollup": { "state": "SUCCESS" } } } ] }
        }))
        .unwrap()
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn snippet_collapses_and_truncates() {
        assert_eq!(snippet(None, 10), None);
        assert_eq!(snippet(Some(""), 10), None);
        assert_eq!(snippet(Some("  \n\t  "), 10), None);
        assert_eq!(snippet(Some("a   b\n c"), 10).as_deref(), Some("a b c"));
        assert_eq!(snippet(Some("abcdefghij"), 5).as_deref(), Some("abcd…"));
    }

    #[test]
    fn upsert_node_auto_adds_an_unregistered_repo_once() {
        let conn = open_memory();
        upsert_node(&conn, &pr_node("PR_1", "acme/widget", 1), "now").unwrap();
        upsert_node(&conn, &pr_node("PR_2", "Acme/Widget", 2), "now").unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM repository WHERE registered_at IS NULL"), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM gh_pr"), 2);
        let (body, ci): (String, String) = conn
            .query_row(
                "SELECT p.body_snippet, s.ci_state FROM gh_pr p JOIN gh_pr_status s ON s.pr_id = p.node_id
                 WHERE p.node_id = 'PR_1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((body.as_str(), ci.as_str()), ("a b", "SUCCESS"));
    }

    #[test]
    fn a_new_head_drops_the_detail_checks() {
        let conn = open_memory();
        upsert_node(&conn, &pr_node("PR_1", "acme/widget", 1), "now").unwrap();
        conn.execute("UPDATE gh_pr_status SET checks_json = '[]'", []).unwrap();
        upsert_node(&conn, &pr_node("PR_1", "acme/widget", 1), "now").unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM gh_pr_status WHERE checks_json IS NOT NULL"), 1);
        let mut moved = pr_node("PR_1", "acme/widget", 1);
        moved.head_ref_oid = Some("h2".into());
        upsert_node(&conn, &moved, "now").unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM gh_pr_status WHERE checks_json IS NOT NULL"), 0);
    }

    #[test]
    fn read_pr_meta_rebuilds_the_header_from_the_mirror() {
        let conn = open_memory();
        assert!(read_pr_meta(&conn, "acme", "widget", 1).unwrap().is_none());
        upsert_node(&conn, &pr_node("PR_1", "acme/widget", 1), "now").unwrap();
        let meta = read_pr_meta(&conn, "acme", "widget", 1).unwrap().unwrap();
        assert_eq!(meta.state, "OPEN");
        assert_eq!(meta.head_ref_oid.as_deref(), Some("h1"));
        assert_eq!(meta.ci_state.as_deref(), Some("SUCCESS"));
        assert_eq!(meta.author.and_then(|a| a.avatar_url).as_deref(), Some("https://a/o.png"));
    }

    #[test]
    fn gc_keeps_referenced_rows_and_drops_old_unreferenced_ones() {
        let conn = open_memory();
        for (id, n) in [("PR_inbox", 1), ("PR_board", 2), ("PR_target", 3), ("PR_orphan", 4), ("PR_fresh", 5)] {
            upsert_node(&conn, &pr_node(id, "acme/widget", n), "2000-01-01T00:00:00Z").unwrap();
        }
        conn.execute("UPDATE gh_pr SET fetched_at = ?1 WHERE node_id = 'PR_fresh'", params![Utc::now().to_rfc3339()])
            .unwrap();
        conn.execute("INSERT INTO inbox_entry (id, type, first_seen_at) VALUES ('PR_inbox', 'pr', 'now')", [])
            .unwrap();
        conn.execute("INSERT INTO board_card (id, type, tracked_at) VALUES ('PR_board', 'pr', 'now')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO target (repo_id, kind, github_pr_number, gh_pr_id, title, base_ref, head_ref, created_at)
             VALUES (1, 'github_pr', 3, 'PR_target', 't', 'main', 'feat', 'now')",
            [],
        )
        .unwrap();
        upsert_node(&conn, &pr_node("PR_gone", "other/repo", 9), "2000-01-01T00:00:00Z").unwrap();

        gc(&conn).unwrap();

        let left: Vec<String> = conn
            .prepare("SELECT node_id FROM gh_pr ORDER BY node_id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(left, vec!["PR_board", "PR_fresh", "PR_inbox", "PR_target"]);
        assert_eq!(
            count(&conn, "SELECT COUNT(*) FROM repository WHERE remote_owner = 'other'"),
            0,
            "auto-added repo with nothing left is dropped"
        );
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM gh_pr_status"), 4, "facets cascade");
    }
}
