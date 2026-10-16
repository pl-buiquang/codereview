//! Board AI overview: a compact text snapshot of the board is sent to a
//! tool-less `claude -p` run and the Markdown answer is stored as one row.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use tauri::State;

use crate::commands::board::board_list_impl;
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

#[tauri::command]
pub async fn board_overview_generate(model: Option<String>, db: State<'_, Db>) -> AppResult<BoardOverview> {
    if crate::tools::claude_path().is_none() {
        return Err(AppError::Other("the `claude` CLI was not found on PATH".into()));
    }
    let (snapshot, card_ids, system_prompt) = {
        let conn = db.0.lock().unwrap();
        let cards = board_list_impl(&conn)?;
        let (snapshot, card_ids) = build_snapshot(&cards, Utc::now());
        (snapshot, card_ids, crate::prompts::resolve(&conn, crate::prompts::BOARD_OVERVIEW)?)
    };
    if card_ids.is_empty() {
        return Err(AppError::Other("no open cards on the board".into()));
    }
    let model = crate::commands::chat::resolve_model(model);
    let run_model = model.clone();
    let turn = tauri::async_runtime::spawn_blocking(move || {
        let cwd = std::env::temp_dir().join("codereview_overview");
        std::fs::create_dir_all(&cwd)?;
        crate::commands::chat::run_claude(crate::commands::chat::ClaudeRun {
            model: &run_model,
            allowed_tools: &[],
            permission_mode: "default",
            system_prompt: Some(&system_prompt),
            prompt_file_tag: "board_overview",
            cwd: &cwd,
            extra_dirs: &[],
            session: None,
            env: Vec::new(),
            extra_args: &["--tools", "", "--effort", "low"],
            replace_system_prompt: true,
            prompt: &snapshot,
        })
    })
    .await
    .map_err(|e| AppError::Other(format!("overview task failed: {e}")))??;

    let overview = BoardOverview {
        markdown: turn.text,
        generated_at: Utc::now().to_rfc3339(),
        model: (!model.is_empty()).then_some(model),
        card_count: card_ids.len() as i64,
        card_ids,
        input_tokens: Some(turn.input_tokens),
        output_tokens: Some(turn.output_tokens),
        cost_usd: Some(turn.cost_usd),
    };
    let conn = db.0.lock().unwrap();
    conn.execute(
        "INSERT INTO board_overview (id, markdown, generated_at, model, card_count, input_tokens, output_tokens, cost_usd, card_ids)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET markdown = excluded.markdown, generated_at = excluded.generated_at,
           model = excluded.model, card_count = excluded.card_count, input_tokens = excluded.input_tokens,
           output_tokens = excluded.output_tokens, cost_usd = excluded.cost_usd, card_ids = excluded.card_ids",
        params![
            overview.markdown,
            overview.generated_at,
            overview.model,
            overview.card_count,
            overview.input_tokens,
            overview.output_tokens,
            overview.cost_usd,
            serde_json::to_string(&overview.card_ids)?
        ],
    )?;
    Ok(overview)
}
