//! Board AI overview: a compact text snapshot of the board is sent to a
//! tool-less `claude -p` run and the Markdown answer is stored as one row.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use tauri::{Manager, State};

use crate::db::models::BoardCardWithExtras;
use crate::db::Db;
use crate::error::{AppError, AppResult};

#[derive(Serialize)]
pub struct BoardOverview {
    pub markdown: String,
    pub generated_at: String,
    pub model: Option<String>,
    pub card_count: i64,
    /// Open card ids the overview was generated from, to flag added/removed cards.
    pub card_ids: Vec<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cost_usd: Option<f64>,
}

fn days_since(ts: &str, now: DateTime<Utc>) -> Option<i64> {
    DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|t| (now - t.with_timezone(&Utc)).num_days().max(0))
}

fn short_repo(repo: &str) -> &str {
    repo.rsplit('/').next().unwrap_or(repo)
}

fn clip(s: &str, max: usize) -> String {
    let one_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        one_line
    } else {
        format!("{}…", one_line.chars().take(max).collect::<String>())
    }
}

/// One line per open card, grouped by repo; returns (snapshot, open card ids).
pub fn build_snapshot(cards: &[BoardCardWithExtras], now: DateTime<Utc>) -> (String, Vec<String>) {
    let number_by_id: HashMap<&str, i64> = cards.iter().map(|c| (c.card.id.as_str(), c.card.number)).collect();
    let mut by_repo: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut done_recent: Vec<String> = Vec::new();
    let mut open_ids: Vec<String> = Vec::new();

    for c in cards {
        let card = &c.card;
        if card.archived_at.is_some() || card.discarded_at.is_some() {
            continue;
        }
        let is_open = card.state.as_deref().is_none_or(|s| s == "open");
        if !is_open {
            if card.closed_at.as_deref().and_then(|t| days_since(t, now)).is_some_and(|d| d <= 7) {
                done_recent.push(format!("{}#{}", short_repo(&card.repo), card.number));
            }
            continue;
        }
        open_ids.push(card.id.clone());

        let mut f: Vec<String> = Vec::new();
        f.push(format!("{} #{}", if card.typ == "pr" { "PR" } else { "ISSUE" }, card.number));
        f.push(c.lane.clone().unwrap_or_else(|| "has_pr".into()));
        if card.typ == "pr" {
            if card.is_mine {
                f.push("mine".into());
            } else if let Some(a) = &card.author_login {
                f.push(format!("by={a}"));
            }
            if card.is_draft {
                f.push("draft".into());
            }
            if let Some(ci) = &card.ci_status {
                f.push(format!("ci={ci}"));
            }
            if let Some(r) = &card.review_decision {
                f.push(format!("review={r}"));
            }
            match card.merge_state_status.as_deref() {
                Some("CLEAN") | Some("UNKNOWN") | None => {}
                Some(m) => f.push(format!("merge={m}")),
            }
            if let (Some(n), Some(pos)) = (card.stack_number, card.stack_position) {
                f.push(format!("stack={n}.{pos}"));
            }
            if card.auto_merge.is_some() {
                f.push("auto_merge".into());
            }
            if card.mergeable.as_deref() == Some("CONFLICTING") && card.merge_state_status.as_deref() != Some("DIRTY") {
                f.push("conflicts".into());
            }
            if let Some(n) = card.pending_threads.filter(|n| *n > 0) {
                f.push(format!("threads={n}"));
            }
        }
        if let Some(d) = days_since(&card.updated_at, now) {
            f.push(format!("upd={d}d"));
        }
        if let Some(d) = days_since(&card.tracked_at, now) {
            f.push(format!("tracked={d}d"));
        }
        let linked: Vec<String> = c
            .linked_issue_ids
            .iter()
            .chain(c.linked_pr_ids.iter())
            .filter_map(|id| number_by_id.get(id.as_str()).map(|n| format!("#{n}")))
            .collect();
        if !linked.is_empty() {
            let key = if card.typ == "pr" { "issue" } else { "prs" };
            f.push(format!("{key}={}", linked.join(",")));
        }
        let mut tags: Vec<&str> = c.meta.iter().chain(c.inherited_meta.iter()).filter_map(|m| m.tag.as_deref()).collect();
        tags.sort_unstable();
        tags.dedup();
        if !tags.is_empty() {
            f.push(format!("tags={}", tags.join(",")));
        }
        let sessions = c.meta.iter().filter(|m| m.kind == "claude_session").count();
        if sessions > 0 {
            f.push(format!("sess={sessions}"));
        }
        for note in c.meta.iter().filter(|m| m.kind == "note" && !m.value.trim().is_empty()).take(2) {
            f.push(format!("note=\"{}\"", clip(&note.value, 80)));
        }
        f.push(format!("\"{}\"", clip(&card.title, 90)));
        by_repo.entry(card.repo.as_str()).or_default().push(f.join(" "));
    }

    let mut out = format!(
        "NOW {} · {} open cards · {} repos\n",
        now.format("%Y-%m-%d"),
        open_ids.len(),
        by_repo.len()
    );
    for (repo, lines) in &by_repo {
        out.push_str(&format!("## {} (ref as {}#N)\n", repo, short_repo(repo)));
        for l in lines {
            out.push_str(l);
            out.push('\n');
        }
    }
    if !done_recent.is_empty() {
        out.push_str(&format!("## done last 7d: {}\n", done_recent.join(", ")));
    }
    (out, open_ids)
}

fn load(conn: &Connection) -> AppResult<Option<BoardOverview>> {
    conn.query_row(
        "SELECT markdown, generated_at, model, card_count, input_tokens, output_tokens, cost_usd, card_ids
         FROM board_overview WHERE id = 1",
        [],
        |r| {
            Ok(BoardOverview {
                markdown: r.get(0)?,
                generated_at: r.get(1)?,
                model: r.get(2)?,
                card_count: r.get(3)?,
                input_tokens: r.get(4)?,
                output_tokens: r.get(5)?,
                cost_usd: r.get(6)?,
                card_ids: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default(),
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

#[tauri::command]
pub fn board_overview_get(db: State<Db>) -> AppResult<Option<BoardOverview>> {
    let conn = db.0.lock().unwrap();
    load(&conn)
}

/// Generate the overview as the built-in `board_overview` routine and wait for it to land.
#[tauri::command]
pub async fn board_overview_generate(
    model: Option<String>,
    app: tauri::AppHandle,
    db: State<'_, Db>,
) -> AppResult<BoardOverview> {
    let routine = crate::commands::routines::builtin(&db, "board_overview")?;
    let ctx = crate::routines::context::ContextRef { kind: "board".into(), id: String::new(), base_paths: vec![] };
    let run = crate::commands::routines::launch(&app, &routine, &ctx, "", model.as_deref().unwrap_or(""))?;
    let run = tauri::async_runtime::spawn_blocking({
        let app = app.clone();
        move || loop {
            let current = crate::routines::manager::get_run(&app.state::<Db>().0.lock().unwrap(), run.id)?;
            if !matches!(current.status.as_str(), "queued" | "running") {
                return Ok::<_, AppError>(current);
            }
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
    })
    .await
    .map_err(|e| AppError::Other(format!("overview task failed: {e}")))??;
    if !matches!(run.status.as_str(), "ok" | "needs_attention") {
        return Err(AppError::Other(
            run.error.unwrap_or_else(|| format!("the overview run ended: {}", run.status)),
        ));
    }
    load(&db.0.lock().unwrap())?.ok_or_else(|| AppError::Other("the overview run produced nothing".into()))
}
