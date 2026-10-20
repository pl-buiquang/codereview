//! Usage log of app-spawned `claude -p` runs. The shared runner (`commands::chat::run_command`)
//! records one `ai_run` row per run, successful or not; the Settings "Usage" section reads it.
//! Codex runs launched by `/local-review --codex` are billed separately and are not tracked.

use std::sync::OnceLock;

use chrono::{DateTime, Duration, Local, SecondsFormat, TimeZone, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::db::Db;
use crate::error::{AppError, AppResult};

static APP: OnceLock<AppHandle> = OnceLock::new();

/// Remember the app handle so the runner can record without every caller passing the DB through.
pub fn init(app: AppHandle) {
    let _ = APP.set(app);
}

/// What a run is for. `feature` is a stable key the Usage section groups by.
#[derive(Clone)]
pub struct RunTag {
    pub feature: &'static str,
    pub variant: Option<String>,
    pub review_id: Option<i64>,
    pub target_id: Option<i64>,
    pub label: String,
    /// Model requested with `--model`; empty when claude picked its own default.
    pub model: String,
}

impl RunTag {
    pub fn new(feature: &'static str) -> Self {
        Self { feature, variant: None, review_id: None, target_id: None, label: String::new(), model: String::new() }
    }

    pub fn variant(mut self, variant: impl Into<String>) -> Self {
        self.variant = Some(variant.into());
        self
    }

    pub fn review(mut self, review_id: i64) -> Self {
        self.review_id = Some(review_id);
        self
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct ModelTokens {
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cost_usd: f64,
}

/// Everything the result envelope reports about one run; all zero when it never produced one.
#[derive(Default, Clone, Debug)]
pub struct RunUsage {
    pub num_turns: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cost_usd: f64,
    pub session_id: Option<String>,
    pub models: Vec<ModelTokens>,
}

/// Fixed-width UTC timestamp, so `started_at` strings compare chronologically.
pub fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn now_stamp() -> String {
    stamp(Utc::now())
}

/// Insert one run and its per-model rows; returns the run id.
pub fn insert_run(
    conn: &mut Connection,
    tag: &RunTag,
    started_at: &str,
    duration_ms: i64,
    usage: &RunUsage,
    error: Option<&str>,
) -> rusqlite::Result<i64> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO ai_run (started_at, duration_ms, feature, variant, review_id, target_id, label,
           requested_model, status, error, num_turns, input_tokens, output_tokens, cache_read_tokens,
           cache_creation_tokens, cost_usd, session_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            started_at,
            duration_ms,
            tag.feature,
            tag.variant,
            tag.review_id,
            tag.target_id,
            tag.label,
            tag.model,
            if error.is_some() { "error" } else { "ok" },
            error,
            usage.num_turns,
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens,
            usage.cache_creation_tokens,
            usage.cost_usd,
            usage.session_id,
        ],
    )?;
    let run_id = tx.last_insert_rowid();

    // Older claude versions omit `modelUsage`; attribute the totals to the requested model.
    let fallback;
    let models: &[ModelTokens] = if usage.models.is_empty() && (usage.cost_usd > 0.0 || usage.input_tokens > 0 || usage.output_tokens > 0) {
        fallback = [ModelTokens {
            model: if tag.model.is_empty() { "unknown".to_string() } else { tag.model.clone() },
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            cache_creation_tokens: usage.cache_creation_tokens,
            cost_usd: usage.cost_usd,
        }];
        &fallback
    } else {
        &usage.models
    };
    for m in models {
        tx.execute(
            "INSERT INTO ai_run_model (run_id, model, input_tokens, output_tokens, cache_read_tokens,
               cache_creation_tokens, cost_usd)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(run_id, model) DO UPDATE SET
               input_tokens = input_tokens + excluded.input_tokens,
               output_tokens = output_tokens + excluded.output_tokens,
               cache_read_tokens = cache_read_tokens + excluded.cache_read_tokens,
               cache_creation_tokens = cache_creation_tokens + excluded.cache_creation_tokens,
               cost_usd = cost_usd + excluded.cost_usd",
            params![run_id, m.model, m.input_tokens, m.output_tokens, m.cache_read_tokens, m.cache_creation_tokens, m.cost_usd],
        )?;
    }
    tx.commit()?;
    Ok(run_id)
}

/// Record a finished run. Never fails the run itself: problems are only logged.
pub fn record(tag: &RunTag, started_at: &str, duration_ms: i64, usage: &RunUsage, error: Option<&str>) {
    let Some(app) = APP.get() else { return };
    let Some(db) = app.try_state::<Db>() else { return };
    let result = {
        let mut conn = db.0.lock().unwrap();
        insert_run(&mut conn, tag, started_at, duration_ms, usage, error)
    };
    match result {
        Ok(id) => {
            log::debug!(
                target: "claude",
                "recorded ai_run {id} ({}{}, ${:.2})",
                tag.feature,
                tag.variant.as_deref().map(|v| format!("/{v}")).unwrap_or_default(),
                usage.cost_usd
            );
            let _ = app.emit("ai-run-recorded", id);
        }
        Err(e) => log::warn!(target: "claude", "failed to record {} usage: {e}", tag.feature),
    }
}

// ---------------------------------------------------------------------------
// Queries

#[derive(Serialize, Default, Debug, PartialEq)]
pub struct UsageTotals {
    pub runs: i64,
    pub errors: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cost_usd: f64,
    pub duration_ms: i64,
}

#[derive(Serialize, Debug)]
pub struct FeatureUsage {
    pub feature: String,
    pub variant: Option<String>,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

#[derive(Serialize, Debug)]
pub struct ModelUsageRow {
    pub model: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

#[derive(Serialize, Debug)]
pub struct AiUsageSummary {
    pub totals: UsageTotals,
    pub by_feature: Vec<FeatureUsage>,
    pub by_model: Vec<ModelUsageRow>,
}

#[derive(Serialize, Debug)]
pub struct AiRun {
    pub id: i64,
    pub started_at: String,
    pub duration_ms: i64,
    pub feature: String,
    pub variant: Option<String>,
    /// Set only while the review still exists, so the UI can open it.
    pub review_id: Option<i64>,
    pub label: String,
    pub requested_model: String,
    pub models: Vec<String>,
    pub status: String,
    pub error: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cost_usd: f64,
    pub repo: Option<String>,
    pub pr_number: Option<i64>,
    pub target_title: Option<String>,
    pub base_ref: Option<String>,
    pub head_ref: Option<String>,
}

/// Lower bound (inclusive) of a range, as a `started_at` stamp; `None` means all time.
pub fn range_start(range: &str, now: DateTime<Local>) -> AppResult<Option<String>> {
    let since = match range {
        "all" => return Ok(None),
        "today" => {
            let midnight = now.date_naive().and_hms_opt(0, 0, 0).expect("valid midnight");
            // A DST gap can make local midnight nonexistent; fall back to 24h ago.
            Local.from_local_datetime(&midnight).earliest().unwrap_or(now - Duration::hours(24))
        }
        "7d" => now - Duration::days(7),
        "30d" => now - Duration::days(30),
        other => return Err(AppError::Other(format!("unknown usage range `{other}`"))),
    };
    Ok(Some(stamp(since.with_timezone(&Utc))))
}

fn totals_at(r: &rusqlite::Row, i: usize) -> rusqlite::Result<UsageTotals> {
    Ok(UsageTotals {
        runs: r.get(i)?,
        errors: r.get(i + 1)?,
        input_tokens: r.get(i + 2)?,
        output_tokens: r.get(i + 3)?,
        cache_read_tokens: r.get(i + 4)?,
        cache_creation_tokens: r.get(i + 5)?,
        cost_usd: r.get(i + 6)?,
        duration_ms: r.get(i + 7)?,
    })
}

const RUN_AGGREGATES: &str = "COUNT(*), COALESCE(SUM(r.status = 'error'), 0),
    COALESCE(SUM(r.input_tokens), 0), COALESCE(SUM(r.output_tokens), 0),
    COALESCE(SUM(r.cache_read_tokens), 0), COALESCE(SUM(r.cache_creation_tokens), 0),
    COALESCE(SUM(r.cost_usd), 0.0), COALESCE(SUM(r.duration_ms), 0)";

pub fn summary(conn: &Connection, since: Option<&str>) -> rusqlite::Result<AiUsageSummary> {
    let totals = conn.query_row(
        &format!("SELECT {RUN_AGGREGATES} FROM ai_run r WHERE (?1 IS NULL OR r.started_at >= ?1)"),
        params![since],
        |r| totals_at(r, 0),
    )?;

    let by_feature = conn
        .prepare(&format!(
            "SELECT r.feature, r.variant, {RUN_AGGREGATES} FROM ai_run r
             WHERE (?1 IS NULL OR r.started_at >= ?1)
             GROUP BY r.feature, r.variant ORDER BY SUM(r.cost_usd) DESC, COUNT(*) DESC"
        ))?
        .query_map(params![since], |r| {
            Ok(FeatureUsage { feature: r.get(0)?, variant: r.get(1)?, totals: totals_at(r, 2)? })
        })?
        .collect::<rusqlite::Result<_>>()?;

    // A run that used several models counts once under each of them; `duration_ms` is per run,
    // so it is left at 0 here rather than double-counted.
    let by_model = conn
        .prepare(
            "SELECT m.model, COUNT(*), COALESCE(SUM(r.status = 'error'), 0),
                COALESCE(SUM(m.input_tokens), 0), COALESCE(SUM(m.output_tokens), 0),
                COALESCE(SUM(m.cache_read_tokens), 0), COALESCE(SUM(m.cache_creation_tokens), 0),
                COALESCE(SUM(m.cost_usd), 0.0), 0
             FROM ai_run_model m JOIN ai_run r ON r.id = m.run_id
             WHERE (?1 IS NULL OR r.started_at >= ?1)
             GROUP BY m.model ORDER BY SUM(m.cost_usd) DESC, COUNT(*) DESC",
        )?
        .query_map(params![since], |r| Ok(ModelUsageRow { model: r.get(0)?, totals: totals_at(r, 1)? }))?
        .collect::<rusqlite::Result<_>>()?;

    Ok(AiUsageSummary { totals, by_feature, by_model })
}

pub fn runs(conn: &Connection, since: Option<&str>, feature: Option<&str>, limit: i64) -> rusqlite::Result<Vec<AiRun>> {
    conn.prepare(
        "SELECT r.id, r.started_at, r.duration_ms, r.feature, r.variant, rev.id, r.label,
            r.requested_model,
            (SELECT group_concat(m.model, char(31)) FROM ai_run_model m WHERE m.run_id = r.id),
            r.status, r.error, r.input_tokens, r.output_tokens, r.cache_read_tokens,
            r.cache_creation_tokens, r.cost_usd,
            COALESCE(repo.friendly_name, repo.remote_owner || '/' || repo.remote_name, repo.local_path),
            t.github_pr_number, t.title, t.base_ref, t.head_ref
         FROM ai_run r
         LEFT JOIN review rev ON rev.id = r.review_id
         LEFT JOIN target t ON t.id = COALESCE(rev.target_id, r.target_id)
         LEFT JOIN repository repo ON repo.id = t.repo_id
         WHERE (?1 IS NULL OR r.started_at >= ?1) AND (?2 IS NULL OR r.feature = ?2)
         ORDER BY r.id DESC LIMIT ?3",
    )?
    .query_map(params![since, feature, limit], |r| {
        let models: Option<String> = r.get(8)?;
        Ok(AiRun {
            id: r.get(0)?,
            started_at: r.get(1)?,
            duration_ms: r.get(2)?,
            feature: r.get(3)?,
            variant: r.get(4)?,
            review_id: r.get(5)?,
            label: r.get(6)?,
            requested_model: r.get(7)?,
            models: models.map(|s| s.split('\u{1f}').map(str::to_string).collect()).unwrap_or_default(),
            status: r.get(9)?,
            error: r.get(10)?,
            input_tokens: r.get(11)?,
            output_tokens: r.get(12)?,
            cache_read_tokens: r.get(13)?,
            cache_creation_tokens: r.get(14)?,
            cost_usd: r.get(15)?,
            repo: r.get(16)?,
            pr_number: r.get(17)?,
            target_title: r.get(18)?,
            base_ref: r.get(19)?,
            head_ref: r.get(20)?,
        })
    })?
    .collect()
}

#[tauri::command]
pub fn ai_usage_summary(range: String, db: State<Db>) -> AppResult<AiUsageSummary> {
    let since = range_start(&range, Local::now())?;
    let conn = db.0.lock().unwrap();
    Ok(summary(&conn, since.as_deref())?)
}

#[tauri::command]
pub fn ai_usage_runs(range: String, feature: Option<String>, limit: Option<i64>, db: State<Db>) -> AppResult<Vec<AiRun>> {
    let since = range_start(&range, Local::now())?;
    let conn = db.0.lock().unwrap();
    Ok(runs(&conn, since.as_deref(), feature.as_deref(), limit.unwrap_or(50).clamp(1, 500))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(name: &str, input: i64, cost: f64) -> ModelTokens {
        ModelTokens { model: name.into(), input_tokens: input, output_tokens: 10, cost_usd: cost, ..Default::default() }
    }

    fn usage(cost: f64, models: Vec<ModelTokens>) -> RunUsage {
        RunUsage { num_turns: 1, input_tokens: 100, output_tokens: 10, cost_usd: cost, models, ..Default::default() }
    }

    #[test]
    fn records_runs_and_aggregates_by_feature_and_model() {
        let mut conn = crate::db::open_memory();
        let chat = RunTag::new("chat").review(7);
        let review = RunTag::new("ai_review").variant("critique").review(7);
        insert_run(&mut conn, &chat, "2026-10-20T10:00:00.000Z", 1000, &usage(1.0, vec![model("opus", 100, 1.0)]), None).unwrap();
        insert_run(
            &mut conn,
            &review,
            "2026-10-20T11:00:00.000Z",
            2000,
            &usage(3.0, vec![model("opus", 80, 2.5), model("haiku", 20, 0.5)]),
            None,
        )
        .unwrap();
        insert_run(&mut conn, &review, "2026-10-20T12:00:00.000Z", 500, &RunUsage::default(), Some("boom")).unwrap();

        let all = summary(&conn, None).unwrap();
        assert_eq!(all.totals.runs, 3);
        assert_eq!(all.totals.errors, 1);
        assert_eq!(all.totals.cost_usd, 4.0);
        assert_eq!(all.totals.duration_ms, 3500);

        let features: Vec<_> = all.by_feature.iter().map(|f| (f.feature.as_str(), f.variant.as_deref(), f.totals.runs)).collect();
        assert_eq!(features, vec![("ai_review", Some("critique"), 2), ("chat", None, 1)]);

        let models: Vec<_> = all.by_model.iter().map(|m| (m.model.as_str(), m.totals.runs, m.totals.cost_usd)).collect();
        assert_eq!(models, vec![("opus", 2, 3.5), ("haiku", 1, 0.5)]);

        let recent = summary(&conn, Some("2026-10-20T11:30:00.000Z")).unwrap();
        assert_eq!(recent.totals.runs, 1);
        assert_eq!(recent.totals.errors, 1);
    }

    #[test]
    fn run_list_filters_by_feature_and_lists_models_newest_first() {
        let mut conn = crate::db::open_memory();
        let review = RunTag::new("ai_review").variant("explain").review(99).label("x");
        insert_run(&mut conn, &RunTag::new("focus"), "2026-10-20T10:00:00.000Z", 1, &usage(0.1, vec![model("opus", 1, 0.1)]), None).unwrap();
        insert_run(
            &mut conn,
            &review,
            "2026-10-20T11:00:00.000Z",
            1,
            &usage(0.2, vec![model("opus", 1, 0.1), model("haiku", 1, 0.1)]),
            None,
        )
        .unwrap();

        let list = runs(&conn, None, None, 50).unwrap();
        assert_eq!(list.iter().map(|r| r.feature.as_str()).collect::<Vec<_>>(), vec!["ai_review", "focus"]);
        assert_eq!(list[0].models.len(), 2);
        assert_eq!(list[0].review_id, None, "review 99 does not exist, so the row is not linkable");

        let only_focus = runs(&conn, None, Some("focus"), 50).unwrap();
        assert_eq!(only_focus.len(), 1);
    }

    #[test]
    fn missing_model_usage_falls_back_to_the_requested_model() {
        let mut conn = crate::db::open_memory();
        let mut tag = RunTag::new("board_overview");
        tag.model = "sonnet".into();
        insert_run(&mut conn, &tag, "2026-10-20T10:00:00.000Z", 1, &usage(0.4, vec![]), None).unwrap();
        insert_run(&mut conn, &RunTag::new("focus"), "2026-10-20T10:01:00.000Z", 1, &usage(0.4, vec![]), None).unwrap();
        // A failed run with no usage adds no model row.
        insert_run(&mut conn, &RunTag::new("focus"), "2026-10-20T10:02:00.000Z", 1, &RunUsage::default(), Some("x")).unwrap();

        let by_model = summary(&conn, None).unwrap().by_model;
        let names: Vec<_> = by_model.iter().map(|m| m.model.as_str()).collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"sonnet") && names.contains(&"unknown"));
    }

    #[test]
    fn range_start_bounds() {
        let now = Local.with_ymd_and_hms(2026, 10, 20, 15, 30, 0).unwrap();
        assert_eq!(range_start("all", now).unwrap(), None);
        let today = range_start("today", now).unwrap().unwrap();
        let midnight = Local.with_ymd_and_hms(2026, 10, 20, 0, 0, 0).unwrap().with_timezone(&Utc);
        assert_eq!(today, stamp(midnight));
        let week = range_start("7d", now).unwrap().unwrap();
        assert_eq!(week, stamp((now - Duration::days(7)).with_timezone(&Utc)));
        assert!(range_start("yesterday", now).is_err());
    }
}
