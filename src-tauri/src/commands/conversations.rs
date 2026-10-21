//! Tauri commands for the Inbox's Conversations tab: review threads and
//! top-level discussions of the PRs/issues I'm notified about or track on the
//! Board, read from the mirror (`gh_pr_threads`, `gh_discussion`) and joined
//! with my own pin / last-seen state (`conversation_state`).

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::db::Db;
use crate::error::AppResult;
use crate::gh::PrThread;
use crate::mirror::fetch::DiscussionComment;
use crate::{gh, mirror};

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConversationFilters {
    /// PR / issue node ids; empty means all.
    pub subject_ids: Vec<String>,
    pub repos: Vec<String>,
    pub pinned: bool,
    pub unread: bool,
    pub unresolved: bool,
    pub show_bots: bool,
    /// Extra bot logins from the settings (lower-case).
    pub bot_logins: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSubject {
    pub id: String,
    /// `pr` or `issue`.
    pub kind: String,
    pub owner: String,
    pub name: String,
    pub number: i64,
    pub title: String,
    pub url: String,
    pub state: Option<String>,
    /// Why GitHub notified me, when it did.
    pub reason: Option<String>,
    pub on_board: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: String,
    /// `thread` or `discussion`.
    pub kind: &'static str,
    pub subject: ConversationSubject,
    pub thread: Option<PrThread>,
    pub comments: Vec<DiscussionComment>,
    pub path: Option<String>,
    pub line: Option<i64>,
    pub diff_hunk: Option<String>,
    pub is_resolved: bool,
    pub is_outdated: bool,
    pub last_comment_at: String,
    /// Url of the latest comment.
    pub url: String,
    pub last_author: Option<String>,
    pub comment_count: usize,
    pub unread_count: usize,
    pub pinned: bool,
    pub is_bot: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubjectFacet {
    pub subject: ConversationSubject,
    pub count: usize,
    pub unread: usize,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationFacets {
    pub subjects: Vec<SubjectFacet>,
    pub repos: Vec<(String, usize)>,
    pub pinned: usize,
    pub unread: usize,
    pub unresolved: usize,
    /// Conversations made only of bot comments.
    pub bots: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationList {
    pub items: Vec<Conversation>,
    /// Conversations matching the filters, before the limit.
    pub total: usize,
    pub facets: ConversationFacets,
}

fn ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Utc))
}

struct SubjectRow {
    subject: ConversationSubject,
    threads_json: Option<String>,
    discussion_json: Option<String>,
    last_read_at: Option<String>,
}

fn read_subjects(conn: &Connection) -> AppResult<Vec<SubjectRow>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, CASE WHEN p.node_id IS NOT NULL THEN 'pr' ELSE 'issue' END,
                r.remote_owner, r.remote_name, COALESCE(p.number, i.number),
                COALESCE(p.title, i.title), COALESCE(p.url, i.url), COALESCE(p.state, i.state),
                n.reason, n.last_read_at,
                EXISTS (SELECT 1 FROM board_card b WHERE b.id = s.id AND b.archived_at IS NULL AND b.discarded_at IS NULL),
                th.threads_json, d.comments_json
         FROM (SELECT subject_node_id AS id FROM gh_notification WHERE subject_node_id IS NOT NULL
               UNION
               SELECT id FROM board_card WHERE archived_at IS NULL AND discarded_at IS NULL) s
         LEFT JOIN gh_pr p ON p.node_id = s.id
         LEFT JOIN gh_issue i ON i.node_id = s.id
         JOIN repository r ON r.id = COALESCE(p.repo_id, i.repo_id)
         LEFT JOIN (SELECT subject_node_id, MAX(reason) AS reason, MAX(last_read_at) AS last_read_at
                    FROM gh_notification GROUP BY subject_node_id) n ON n.subject_node_id = s.id
         LEFT JOIN gh_pr_threads th ON th.pr_id = s.id
         LEFT JOIN gh_discussion d ON d.subject_id = s.id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(SubjectRow {
                subject: ConversationSubject {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    owner: r.get(2)?,
                    name: r.get(3)?,
                    number: r.get(4)?,
                    title: r.get(5)?,
                    url: r.get(6)?,
                    state: r.get(7)?,
                    reason: r.get(8)?,
                    on_board: r.get(10)?,
                },
                last_read_at: r.get(9)?,
                threads_json: r.get(11)?,
                discussion_json: r.get(12)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// What the flattening needs to know about one comment.
struct Meta {
    created_at: String,
    author: Option<String>,
    is_bot: bool,
}

struct Ctx {
    viewer: Option<String>,
    baseline: Option<DateTime<Utc>>,
    bot_logins: HashSet<String>,
    db_bots: HashSet<String>,
}

impl Ctx {
    fn is_bot(&self, a: Option<&gh::PrActor>) -> bool {
        let Some(a) = a else { return false };
        let login = a.login.as_deref().map(str::to_lowercase);
        a.typename.as_deref() == Some("Bot")
            || login.as_ref().is_some_and(|l| self.bot_logins.contains(l) || self.db_bots.contains(l))
    }

    fn meta(&self, created_at: &str, a: Option<&gh::PrActor>) -> Meta {
        Meta { created_at: created_at.to_string(), author: a.and_then(|a| a.login.clone()), is_bot: self.is_bot(a) }
    }

    fn unread(&self, seen: Option<DateTime<Utc>>, metas: &[Meta]) -> usize {
        metas
            .iter()
            .filter(|m| {
                let mine = self.viewer.as_deref().is_some_and(|v| m.author.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(v)));
                !mine && ts(&m.created_at).is_some_and(|t| seen.is_none_or(|s| t > s))
            })
            .count()
    }
}

pub fn list_conversations_impl(conn: &Connection, f: &ConversationFilters, limit: usize) -> AppResult<ConversationList> {
    let ctx = Ctx {
        viewer: mirror::store::viewer_login(conn).map(|v| v.to_lowercase()),
        baseline: crate::commands::inbox::get_meta(conn, "conversations_seen_baseline").and_then(|t| ts(&t)),
        bot_logins: f.bot_logins.iter().map(|l| l.to_lowercase()).collect(),
        db_bots: conn
            .prepare("SELECT LOWER(login) FROM gh_user WHERE is_bot = 1")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?,
    };
    let state: HashMap<String, (bool, Option<String>)> = conn
        .prepare("SELECT id, pinned_at IS NOT NULL, last_seen_at FROM conversation_state")?
        .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
        .collect::<rusqlite::Result<_>>()?;

    let mut all: Vec<Conversation> = Vec::new();
    let mut bots = 0usize;
    for row in read_subjects(conn)? {
        let threads: Vec<PrThread> = row.threads_json.as_deref().and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default();
        let discussion: Vec<DiscussionComment> =
            row.discussion_json.as_deref().and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default();
        let notif_read = row.last_read_at.as_deref().and_then(ts);
        let seen_of = |id: &str| {
            let own = state.get(id).and_then(|(_, s)| s.as_deref()).and_then(ts);
            [own.or(ctx.baseline), notif_read].into_iter().flatten().max()
        };
        let pinned_of = |id: &str| state.get(id).is_some_and(|(p, _)| *p);

        for t in threads {
            let Some(last) = t.comments.last() else { continue };
            let metas: Vec<Meta> = t.comments.iter().map(|c| ctx.meta(&c.created_at, c.author.as_ref())).collect();
            let is_bot = metas.iter().all(|m| m.is_bot);
            if is_bot {
                bots += 1;
                if !f.show_bots {
                    continue;
                }
            }
            all.push(Conversation {
                id: t.id.clone(),
                kind: "thread",
                subject: row.subject.clone(),
                path: t.path.clone(),
                line: t.line.or(t.original_line),
                diff_hunk: t.comments.first().and_then(|c| c.diff_hunk.clone()),
                is_resolved: t.is_resolved,
                is_outdated: t.is_outdated,
                last_comment_at: last.created_at.clone(),
                url: last.url.clone(),
                last_author: last.author.as_ref().and_then(|a| a.login.clone()),
                comment_count: t.comments.len(),
                unread_count: ctx.unread(seen_of(&t.id), &metas),
                pinned: pinned_of(&t.id),
                is_bot,
                comments: Vec::new(),
                thread: Some(t),
            });
        }

        if !discussion.is_empty() {
            let id = format!("discussion:{}", row.subject.id);
            let all_bot = discussion.iter().all(|c| ctx.is_bot(c.author.as_ref()));
            if all_bot {
                bots += 1;
            }
            let visible: Vec<DiscussionComment> = discussion
                .into_iter()
                .filter(|c| f.show_bots || !ctx.is_bot(c.author.as_ref()))
                .collect();
            if let Some(last) = visible.last() {
                let metas: Vec<Meta> = visible.iter().map(|c| ctx.meta(&c.created_at, c.author.as_ref())).collect();
                all.push(Conversation {
                    id: id.clone(),
                    kind: "discussion",
                    subject: row.subject.clone(),
                    thread: None,
                    path: None,
                    line: None,
                    diff_hunk: None,
                    is_resolved: false,
                    is_outdated: false,
                    last_comment_at: last.created_at.clone(),
                    url: last.url.clone(),
                    last_author: last.author.as_ref().and_then(|a| a.login.clone()),
                    comment_count: visible.len(),
                    unread_count: ctx.unread(seen_of(&id), &metas),
                    pinned: pinned_of(&id),
                    is_bot: all_bot,
                    comments: visible,
                });
            }
        }
    }

    let mut facets = ConversationFacets { bots, ..Default::default() };
    let mut by_subject: HashMap<&str, SubjectFacet> = HashMap::new();
    let mut by_repo: HashMap<String, usize> = HashMap::new();
    for c in &all {
        let e = by_subject
            .entry(c.subject.id.as_str())
            .or_insert_with(|| SubjectFacet { subject: c.subject.clone(), count: 0, unread: 0 });
        e.count += 1;
        e.unread += usize::from(c.unread_count > 0);
        *by_repo.entry(format!("{}/{}", c.subject.owner, c.subject.name)).or_default() += 1;
        facets.pinned += usize::from(c.pinned);
        facets.unread += usize::from(c.unread_count > 0);
        facets.unresolved += usize::from(c.kind == "thread" && !c.is_resolved);
    }
    facets.subjects = by_subject.into_values().collect();
    facets.subjects.sort_by(|a, b| a.subject.title.to_lowercase().cmp(&b.subject.title.to_lowercase()));
    facets.repos = by_repo.into_iter().collect();
    facets.repos.sort();

    let repos: HashSet<String> = f.repos.iter().map(|r| r.to_lowercase()).collect();
    let mut items: Vec<Conversation> = all
        .into_iter()
        .filter(|c| f.subject_ids.is_empty() || f.subject_ids.contains(&c.subject.id))
        .filter(|c| repos.is_empty() || repos.contains(&format!("{}/{}", c.subject.owner, c.subject.name).to_lowercase()))
        .filter(|c| !f.pinned || c.pinned)
        .filter(|c| !f.unread || c.unread_count > 0)
        .filter(|c| !f.unresolved || (c.kind == "thread" && !c.is_resolved))
        .collect();
    items.sort_by(|a, b| b.last_comment_at.cmp(&a.last_comment_at));
    let total = items.len();
    items.truncate(limit.max(1));
    Ok(ConversationList { items, total, facets })
}

#[tauri::command]
pub fn list_conversations(filters: ConversationFilters, limit: Option<usize>, db: State<Db>) -> AppResult<ConversationList> {
    list_conversations_impl(&db.0.lock().unwrap(), &filters, limit.unwrap_or(100))
}

pub fn set_pinned_impl(conn: &Connection, id: &str, subject_id: &str, pinned: bool) -> AppResult<()> {
    conn.execute(
        "INSERT INTO conversation_state (id, subject_id, pinned_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET pinned_at = excluded.pinned_at",
        params![id, subject_id, pinned.then(|| Utc::now().to_rfc3339())],
    )?;
    Ok(())
}

pub fn set_seen_impl(conn: &Connection, id: &str, subject_id: &str, seen_at: Option<&str>) -> AppResult<()> {
    let seen_at = seen_at.map(str::to_string).unwrap_or_else(|| Utc::now().to_rfc3339());
    conn.execute(
        "INSERT INTO conversation_state (id, subject_id, last_seen_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET last_seen_at = excluded.last_seen_at",
        params![id, subject_id, seen_at],
    )?;
    Ok(())
}

#[tauri::command]
pub fn set_conversation_pinned(id: String, subject_id: String, pinned: bool, db: State<Db>) -> AppResult<()> {
    set_pinned_impl(&db.0.lock().unwrap(), &id, &subject_id, pinned)
}

/// Mark a conversation read up to `seen_at` (the time of its latest comment),
/// or unread by passing an earlier time. `None` means now.
#[tauri::command]
pub fn set_conversation_seen(id: String, subject_id: String, seen_at: Option<String>, db: State<Db>) -> AppResult<()> {
    set_seen_impl(&db.0.lock().unwrap(), &id, &subject_id, seen_at.as_deref())
}

/// Post a top-level comment on a PR/issue and refresh its discussion facet.
/// `async` so the `gh` calls stay off the UI thread.
#[tauri::command]
pub async fn reply_to_discussion(
    subject_id: String,
    owner: String,
    name: String,
    number: i64,
    body: String,
    app: AppHandle,
    db: State<'_, Db>,
) -> AppResult<()> {
    gh::comment_on_issue(&owner, &name, number, &body)?;
    let ids = [subject_id.clone()];
    if let Some(Some(d)) = mirror::fetch::discussions(&ids).remove(&subject_id) {
        let updated_at = d.updated_at.clone();
        let conn = db.0.lock().unwrap();
        mirror::store::upsert_discussion(&conn, &subject_id, updated_at.as_deref(), &d.into_comments(), &Utc::now().to_rfc3339())?;
    }
    crate::commands::gh::notify_mirror(&app, &subject_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;
    use crate::mirror::Node;

    fn pr_node(id: &str, number: i64) -> Node {
        serde_json::from_value(serde_json::json!({
            "__typename": "PullRequest", "id": id, "number": number, "title": format!("PR {number}"), "url": "u",
            "bodyText": "", "state": "OPEN", "updatedAt": "2026-01-01T00:00:00Z",
            "headRefName": "feat", "baseRefName": "main", "headRefOid": "h1",
            "repository": { "nameWithOwner": "acme/widget" }
        }))
        .unwrap()
    }

    fn thread(id: &str, comments: &[(&str, &str, &str)]) -> serde_json::Value {
        serde_json::json!({
            "id": id, "isResolved": false, "isOutdated": false, "isCollapsed": false, "path": "src/a.rs", "line": 3,
            "comments": comments.iter().enumerate().map(|(i, (login, typename, at))| serde_json::json!({
                "id": format!("{id}-c{i}"), "databaseId": i, "author": { "login": login, "__typename": typename },
                "body": "b", "createdAt": at, "url": format!("https://x/{id}/{i}"), "diffHunk": "@@ -1 +1 @@\n+x", "outdated": false
            })).collect::<Vec<_>>()
        })
    }

    fn seed(conn: &Connection, threads: Vec<serde_json::Value>) {
        mirror::store::upsert_node(conn, &pr_node("PR_1", 1), "2026-01-01T00:00:00Z").unwrap();
        conn.execute("INSERT INTO board_card (id, type, tracked_at) VALUES ('PR_1', 'pr', 'now')", []).unwrap();
        conn.execute(
            "INSERT INTO gh_pr_threads (pr_id, threads_json, fetched_at) VALUES ('PR_1', ?1, 'now')",
            params![serde_json::Value::Array(threads).to_string()],
        )
        .unwrap();
        conn.execute("INSERT OR REPLACE INTO meta (key, value) VALUES ('viewer_login', 'me')", []).unwrap();
        conn.execute("UPDATE meta SET value = '2026-01-01T00:00:00Z' WHERE key = 'conversations_seen_baseline'", []).unwrap();
    }

    fn list(conn: &Connection, f: ConversationFilters, limit: usize) -> ConversationList {
        list_conversations_impl(conn, &f, limit).unwrap()
    }

    #[test]
    fn unread_counts_comments_after_the_baseline_that_are_not_mine() {
        let conn = open_memory();
        seed(&conn, vec![
            thread("T_old", &[("bob", "User", "2025-12-31T00:00:00Z")]),
            thread("T_new", &[("bob", "User", "2026-01-02T00:00:00Z"), ("me", "User", "2026-01-03T00:00:00Z")]),
        ]);
        let got = list(&conn, ConversationFilters::default(), 10);
        let unread: Vec<(&str, usize)> = got.items.iter().map(|c| (c.id.as_str(), c.unread_count)).collect();
        assert_eq!(unread, vec![("T_new", 1), ("T_old", 0)]);
        assert_eq!(got.facets.unread, 1);
    }

    #[test]
    fn seen_state_clears_unread_and_unread_filter_follows() {
        let conn = open_memory();
        seed(&conn, vec![thread("T", &[("bob", "User", "2026-01-02T00:00:00Z")])]);
        let unread = ConversationFilters { unread: true, ..Default::default() };
        assert_eq!(list(&conn, ConversationFilters { unread: true, ..Default::default() }, 10).total, 1);
        set_seen_impl(&conn, "T", "PR_1", Some("2026-01-02T00:00:00Z")).unwrap();
        assert_eq!(list(&conn, unread, 10).total, 0);
        set_seen_impl(&conn, "T", "PR_1", Some("1970-01-01T00:00:00Z")).unwrap();
        assert_eq!(list(&conn, ConversationFilters { unread: true, ..Default::default() }, 10).total, 1);
    }

    #[test]
    fn bot_only_conversations_are_hidden_unless_shown() {
        let conn = open_memory();
        seed(&conn, vec![
            thread("T_bot", &[("copilot", "Bot", "2026-01-02T00:00:00Z")]),
            thread("T_cfg", &[("sonar-x", "User", "2026-01-02T00:00:00Z")]),
            thread("T_human", &[("bob", "User", "2026-01-02T00:00:00Z"), ("copilot", "Bot", "2026-01-03T00:00:00Z")]),
        ]);
        let hidden = ConversationFilters { bot_logins: vec!["Sonar-X".into()], ..Default::default() };
        let got = list(&conn, hidden, 10);
        assert_eq!(got.items.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["T_human"]);
        assert_eq!(got.facets.bots, 2);
        let shown = ConversationFilters { show_bots: true, ..Default::default() };
        assert_eq!(list(&conn, shown, 10).total, 3);
    }

    #[test]
    fn discussion_strips_bot_comments_and_pinned_filter_applies() {
        let conn = open_memory();
        seed(&conn, vec![]);
        let comments = serde_json::json!([
            { "id": "C1", "kind": "comment", "author": { "login": "sonar", "__typename": "Bot" }, "body": "gate",
              "createdAt": "2026-01-02T00:00:00Z", "url": "u1" },
            { "id": "C2", "kind": "comment", "author": { "login": "bob", "__typename": "User" }, "body": "hi",
              "createdAt": "2026-01-03T00:00:00Z", "url": "u2" }]);
        conn.execute("INSERT INTO gh_discussion (subject_id, comments_json, fetched_at) VALUES ('PR_1', ?1, 'now')", params![comments.to_string()]).unwrap();
        let got = list(&conn, ConversationFilters::default(), 10);
        assert_eq!(got.items.len(), 1);
        assert_eq!((got.items[0].kind, got.items[0].comments.len()), ("discussion", 1));
        assert_eq!(list(&conn, ConversationFilters { pinned: true, ..Default::default() }, 10).total, 0);
        set_pinned_impl(&conn, "discussion:PR_1", "PR_1", true).unwrap();
        assert_eq!(list(&conn, ConversationFilters { pinned: true, ..Default::default() }, 10).total, 1);
        set_pinned_impl(&conn, "discussion:PR_1", "PR_1", false).unwrap();
        assert_eq!(list(&conn, ConversationFilters { pinned: true, ..Default::default() }, 10).total, 0);
    }

    #[test]
    fn limit_applies_after_filters_and_total_counts_all() {
        let conn = open_memory();
        seed(&conn, (0..5).map(|i| thread(&format!("T{i}"), &[("bob", "User", &format!("2026-01-0{}T00:00:00Z", i + 2))])).collect());
        let got = list(&conn, ConversationFilters::default(), 2);
        assert_eq!((got.items.len(), got.total), (2, 5));
        assert_eq!(got.items[0].id, "T4", "newest first");
        assert_eq!(got.facets.subjects.len(), 1);
    }
}
