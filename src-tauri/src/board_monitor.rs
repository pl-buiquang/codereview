//! The board's auto-merge monitor: for approved PRs that have GitHub
//! auto-merge on, keep them mergeable by updating a BEHIND branch and
//! rerunning failed CI (GitHub does the merge itself). Free of Tauri so the
//! scheduler and a future `cr` one-shot can both drive it.

use rusqlite::Connection;
use serde::Serialize;

use crate::commands::board::{refetch_card, CARD_VIEW};
use crate::db::models::BoardCardRow;
use crate::db::Db;
use crate::error::AppResult;
use crate::gh;

/// Automatic reruns stop once a workflow run has been attempted this many times.
const MAX_ATTEMPTS: i64 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub card_id: String,
    pub owner: String,
    pub name: String,
    pub number: i64,
    pub merge_state_status: Option<String>,
    pub viewer_can_update_branch: bool,
    pub ci_status: Option<String>,
    pub stacked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Updated,
    Reran,
    GaveUp,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct MonitorAction {
    pub card_id: String,
    pub repo: String,
    pub number: i64,
    pub kind: ActionKind,
    pub detail: String,
}

/// Open, non-draft, tracked PRs that are approved and have auto-merge on.
pub fn candidates(conn: &Connection) -> AppResult<Vec<Candidate>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT * FROM {CARD_VIEW}
         WHERE type = 'pr' AND state = 'open' AND is_draft = 0
           AND discarded_at IS NULL AND archived_at IS NULL
           AND auto_merge IS NOT NULL AND review_decision = 'APPROVED'"
    ))?;
    let cards = stmt.query_map([], BoardCardRow::from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(cards
        .into_iter()
        .filter_map(|c| {
            let (owner, name) = c.repo.split_once('/')?;
            Some(Candidate {
                owner: owner.to_string(),
                name: name.to_string(),
                card_id: c.id,
                number: c.number,
                merge_state_status: c.merge_state_status,
                viewer_can_update_branch: c.viewer_can_update_branch,
                ci_status: c.ci_status,
                stacked: c.stack_number.is_some(),
            })
        })
        .collect())
}

fn ci_failing(c: &Candidate) -> bool {
    matches!(c.ci_status.as_deref(), Some("FAILURE" | "ERROR"))
}

fn act(c: &Candidate, kind: ActionKind, detail: impl Into<String>) -> MonitorAction {
    MonitorAction {
        card_id: c.card_id.clone(),
        repo: format!("{}/{}", c.owner, c.name),
        number: c.number,
        kind,
        detail: detail.into(),
    }
}

fn check(c: &Candidate, actions: &mut Vec<MonitorAction>) -> AppResult<bool> {
    let repo = format!("{}/{}", c.owner, c.name);
    if c.merge_state_status.as_deref() == Some("BEHIND") && c.viewer_can_update_branch && !c.stacked {
        gh::update_pr_branch(&c.owner, &c.name, c.number, false)?;
        actions.push(act(c, ActionKind::Updated, "branch updated"));
        return Ok(true);
    }
    if !ci_failing(c) {
        return Ok(false);
    }
    let mut touched = false;
    for run in gh::failing_workflow_runs(&c.owner, &c.name, c.number)? {
        let attempt = gh::run_attempt(&repo, run.run_id)?;
        if attempt < MAX_ATTEMPTS {
            gh::rerun_run(&repo, run.run_id, true)?;
            actions.push(act(c, ActionKind::Reran, format!("{} (attempt {attempt})", run.workflow)));
            touched = true;
        } else {
            actions.push(act(c, ActionKind::GaveUp, format!("{} failed {attempt} times", run.workflow)));
        }
    }
    Ok(touched)
}

/// One pass over the candidates. `gh` calls run unlocked; a per-PR failure is
/// recorded as an `Error` action and never aborts the pass.
pub fn run(db: &Db) -> AppResult<Vec<MonitorAction>> {
    let candidates = candidates(&db.0.lock().unwrap())?;
    let mut actions = Vec::new();
    for c in &candidates {
        match check(c, &mut actions) {
            Ok(true) => {
                if let Err(e) = refetch_card(db, &c.owner, &c.name, c.number) {
                    actions.push(act(c, ActionKind::Error, format!("refetch failed: {e}")));
                }
            }
            Ok(false) => {}
            Err(e) => actions.push(act(c, ActionKind::Error, e.to_string())),
        }
    }
    Ok(actions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;
    use crate::mirror;
    use rusqlite::params;

    fn seed_pr(conn: &Connection, id: &str, number: i64) {
        let repo_id = mirror::store::ensure_repo(conn, "owner", "repo", "now").unwrap();
        conn.execute(
            "INSERT INTO gh_pr (node_id, repo_id, number, title, url, gh_updated_at, fetched_at, state, is_draft, review_decision)
             VALUES (?1, ?2, ?3, 't', 'u', 'now', 'now', 'open', 0, 'APPROVED')",
            params![id, repo_id, number],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO gh_pr_status (pr_id, auto_merge_method, merge_state_status, ci_state, fetched_at)
             VALUES (?1, 'SQUASH', 'BEHIND', 'FAILURE', 'now')",
            params![id],
        )
        .unwrap();
        conn.execute("INSERT INTO board_card (id, type, tracked_at) VALUES (?1, 'pr', 'now')", params![id]).unwrap();
    }

    fn ids(conn: &Connection) -> Vec<String> {
        candidates(conn).unwrap().into_iter().map(|c| c.card_id).collect()
    }

    #[test]
    fn selects_approved_open_prs_with_auto_merge() {
        let conn = open_memory();
        seed_pr(&conn, "pr1", 1);
        let found = candidates(&conn).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].owner.as_str(), found[0].name.as_str(), found[0].number), ("owner", "repo", 1));
        assert_eq!(found[0].merge_state_status.as_deref(), Some("BEHIND"));
        assert!(ci_failing(&found[0]));
    }

    #[test]
    fn skips_prs_the_monitor_must_not_touch() {
        let conn = open_memory();
        for (i, id) in ["draft", "closed", "discarded", "archived", "no_auto", "unapproved"].iter().enumerate() {
            seed_pr(&conn, id, i as i64 + 10);
        }
        conn.execute("UPDATE gh_pr SET is_draft = 1 WHERE node_id = 'draft'", []).unwrap();
        conn.execute("UPDATE gh_pr SET state = 'closed' WHERE node_id = 'closed'", []).unwrap();
        conn.execute("UPDATE board_card SET discarded_at = 'now' WHERE id = 'discarded'", []).unwrap();
        conn.execute("UPDATE board_card SET archived_at = 'now' WHERE id = 'archived'", []).unwrap();
        conn.execute("UPDATE gh_pr_status SET auto_merge_method = NULL WHERE pr_id = 'no_auto'", []).unwrap();
        conn.execute("UPDATE gh_pr SET review_decision = 'REVIEW_REQUIRED' WHERE node_id = 'unapproved'", []).unwrap();
        seed_pr(&conn, "ok", 1);
        assert_eq!(ids(&conn), vec!["ok".to_string()]);
    }

    #[test]
    fn stacked_prs_are_flagged() {
        let conn = open_memory();
        seed_pr(&conn, "pr1", 1);
        conn.execute("UPDATE gh_pr_status SET stack_number = 5", []).unwrap();
        assert!(candidates(&conn).unwrap()[0].stacked);
    }
}
