//! Rust hooks of the built-in routines that feed app features (`routine.handler`): the board
//! overview, focus generation and the AI review layers. A handler can veto a launch and set up its
//! side effects (`pre_launch`), shape the run (`prepare`: prompt variables, working directory,
//! extra directories and tools) and post-process the result (`finish`). Plain routines have none.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::Utc;
use rusqlite::params;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use super::context::{ContextRef, Light};
use super::manager::{get_run, routine_settings, RoutineRun};
use super::Routine;
use crate::ai_usage::{RunTag, RunUsage};
use crate::commands::chat::resolve_model;
use crate::commands::focus::{begin_generation, finish_generation, review_local_path};
use crate::commands::{board::board_list_impl, overview, review::ensure_draft};
use crate::db::{Db, DbPath};
use crate::image_diff::{report, store};
use crate::error::{AppError, AppResult};

const AI_REVIEW_PREFIX: &str = "ai_review_";
pub const IMAGE_DIFF_REPORT_EVENT: &str = "image-diff-report-updated";

/// What a handler adds to a run before it spawns.
#[derive(Default)]
pub struct Prep {
    /// Extra prompt template variables.
    pub vars: Vec<(String, String)>,
    /// Working directory, when the context's own doesn't fit.
    pub cwd: Option<PathBuf>,
    pub extra_dirs: Vec<PathBuf>,
    /// Allowed-tool rules added to the routine's.
    pub extra_tools: Vec<String>,
    /// System prompt used as-is instead of the routine's rendered one.
    pub system: Option<String>,
    pub system_suffix: String,
    /// Scratch directories removed once the process has ended.
    pub cleanup: Vec<PathBuf>,
}

#[derive(Clone, Serialize)]
struct AiReviewUpdated {
    review_id: i64,
    layer: String,
    status: &'static str,
    error: Option<String>,
    summary: Option<String>,
}

#[derive(Clone, Serialize)]
struct FocusUpdated {
    review_id: i64,
}

#[derive(Clone, Serialize)]
struct ImageDiffReportUpdated {
    review_id: i64,
    status: &'static str,
    error: Option<String>,
}

fn emit_report(app: &AppHandle, review_id: i64, status: &'static str, error: Option<String>) {
    let _ = app.emit(IMAGE_DIFF_REPORT_EVENT, ImageDiffReportUpdated { review_id, status, error });
}

/// Open-card ids each overview run was generated from, until the run ends.
static OVERVIEW_IDS: Mutex<Option<HashMap<i64, Vec<String>>>> = Mutex::new(None);

/// The model a run asks for: the launch override, else the routine's, else the app-wide default.
pub fn effective_model(run: &RoutineRun, routine: &Routine) -> String {
    resolve_model(Some(if run.model.is_empty() { routine.model.clone() } else { run.model.clone() }))
}

fn review_id(ctx: &ContextRef) -> AppResult<i64> {
    ctx.id.parse().map_err(|_| AppError::Other("bad review id".into()))
}

fn layer(routine: &Routine) -> String {
    routine.key.strip_prefix(AI_REVIEW_PREFIX).unwrap_or(&routine.key).to_string()
}

/// Usage-log tag: converted jobs keep the feature names they had before they ran as routines.
pub fn usage_tag(routine: &Routine, run: &RoutineRun) -> RunTag {
    let review = review_id(&run.context).ok();
    let tag = match routine.handler.as_deref() {
        Some("overview") => RunTag::new("board_overview").label("Board overview"),
        Some("focus") => RunTag::new("focus"),
        Some("ai_review") => RunTag::new("ai_review").variant(layer(routine)),
        Some("image_diff_report") => RunTag::new("image_diff_report").label("Image diff report"),
        _ => {
            return RunTag::new("routine")
                .variant(routine.key.clone())
                .label(format!("{} · {}", routine.name, run.context_label))
        }
    };
    match review {
        Some(id) if routine.handler.as_deref() != Some("overview") => tag.review(id),
        _ => tag,
    }
}

/// Checks and side effects of a launch. Runs before the run row exists; `rollback` undoes it when
/// the launch then fails.
pub fn pre_launch(app: &AppHandle, routine: &Routine, ctx: &ContextRef, light: &Light, model: &str) -> AppResult<()> {
    let db = app.state::<Db>();
    match routine.handler.as_deref() {
        Some("overview") => {
            let conn = db.0.lock().unwrap();
            let (_, ids) = overview::build_snapshot(&board_list_impl(&conn)?, Utc::now());
            if ids.is_empty() {
                return Err(AppError::Other("no open cards on the board".into()));
            }
        }
        Some("focus") => {
            let id = review_id(ctx)?;
            let model = resolve_model(Some(if model.is_empty() { routine.model.clone() } else { model.to_string() }));
            begin_generation(&db.0.lock().unwrap(), id, &model)?;
            let _ = app.emit("focus-updated", FocusUpdated { review_id: id });
        }
        Some("ai_review") => {
            let id = review_id(ctx)?;
            let conn = db.0.lock().unwrap();
            ensure_draft(&conn, id)?;
            // One layer per review at a time, whichever layer it is.
            let running: Option<String> = conn
                .query_row(
                    "SELECT ro.key FROM routine_run r JOIN routine ro ON ro.id = r.routine_id
                     WHERE ro.handler = 'ai_review' AND r.context_key = ?1 AND r.status IN ('queued', 'running')",
                    params![light.key],
                    |r| r.get(0),
                )
                .ok();
            if let Some(key) = running {
                let running = key.strip_prefix(AI_REVIEW_PREFIX).unwrap_or(&key).to_string();
                return Err(AppError::Other(format!("an AI review ({running}) is already running for this review")));
            }
            drop(conn);
            emit_ai_review(app, id, layer(routine), "running", None, None);
        }
        Some("image_diff_report") => {
            let id = review_id(ctx)?;
            let dir = image_diff_dir(app, id);
            let manifest = store::read_manifest(&dir)
                .ok_or_else(|| AppError::Other("generate the image diffs first".into()))?;
            if !manifest.files.iter().any(|f| f.skipped.is_none() && f.mismatch_px > 0) {
                return Err(AppError::Other("no changed pixels to describe".into()));
            }
            if active_image_diff_report(&db.0.lock().unwrap(), id) {
                return Err(AppError::Other("an image diff report is already running for this review".into()));
            }
            emit_report(app, id, "running", None);
        }
        _ => {}
    }
    Ok(())
}

fn image_diff_dir(app: &AppHandle, review_id: i64) -> PathBuf {
    store::cache_dir(&app.state::<DbPath>().0, review_id)
}

/// The launch failed after `pre_launch` succeeded.
pub fn rollback(app: &AppHandle, routine: &Routine, ctx: &ContextRef, error: &str) {
    let Ok(id) = review_id(ctx) else { return };
    match routine.handler.as_deref() {
        Some("focus") => {
            let _ = finish_generation(&app.state::<Db>().0.lock().unwrap(), id, Some(error.to_string()));
            let _ = app.emit("focus-updated", FocusUpdated { review_id: id });
        }
        Some("ai_review") => emit_ai_review(app, id, layer(routine), "failed", Some(error.to_string()), None),
        Some("image_diff_report") => emit_report(app, id, "failed", Some(error.to_string())),
        _ => {}
    }
}

fn emit_ai_review(
    app: &AppHandle,
    review_id: i64,
    layer: String,
    status: &'static str,
    error: Option<String>,
    summary: Option<String>,
) {
    let _ = app.emit("ai-review-updated", AiReviewUpdated { review_id, layer, status, error, summary });
}

fn scratch_for(prefix: &str, id: i64) -> AppResult<PathBuf> {
    // Canonical so path rules match (macOS /var → /private/var).
    let dir = std::env::temp_dir().canonicalize()?.join(format!("{prefix}-{id}"));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Working directory of review-scoped jobs: the linked clone, else a throwaway directory.
fn review_cwd(app: &AppHandle, id: i64, tag: &str) -> AppResult<PathBuf> {
    let local = review_local_path(&app.state::<Db>().0.lock().unwrap(), id)?;
    match local {
        Some(p) if std::path::Path::new(&p).exists() => Ok(PathBuf::from(p)),
        _ => {
            let dir = std::env::temp_dir().join(format!("codereview_{tag}_{id}"));
            std::fs::create_dir_all(&dir)?;
            Ok(dir)
        }
    }
}

pub fn prepare(app: &AppHandle, routine: &Routine, run: &RoutineRun) -> AppResult<Prep> {
    let mut prep = Prep::default();
    match routine.handler.as_deref() {
        Some("overview") => {
            let (snapshot, ids) = {
                let db = app.state::<Db>();
                let conn = db.0.lock().unwrap();
                overview::build_snapshot(&board_list_impl(&conn)?, Utc::now())
            };
            if ids.is_empty() {
                return Err(AppError::Other("no open cards on the board".into()));
            }
            OVERVIEW_IDS.lock().unwrap().get_or_insert_with(HashMap::new).insert(run.id, ids);
            prep.vars.push(("snapshot".into(), snapshot));
            prep.cwd = Some(std::env::temp_dir().join("codereview_overview"));
            std::fs::create_dir_all(prep.cwd.as_ref().unwrap())?;
        }
        Some("focus") => {
            let id = review_id(&run.context)?;
            // Plans go through a file: Claude Code refuses Bash commands that carry inline JSON.
            let scratch = scratch_for("codereview-focus", id)?;
            let plan = scratch.join("plan.json").display().to_string();
            prep.extra_tools.push(format!("Edit(/{}/**)", scratch.display()));
            prep.system_suffix = format!(
                "\n\n---\n\nApp mode. Review id = {id}. Do not create or open reviews. \
                 The `cr` CLI on PATH already points at the app's database. \
                 Write the plan with the Write tool to `{plan}` (the only writable path), then run \
                 `cr focus set {id} --file {plan}`."
            );
            prep.cwd = Some(review_cwd(app, id, "focus")?);
            prep.extra_dirs.push(scratch.clone());
            prep.cleanup.push(scratch);
        }
        Some("ai_review") => {
            let id = review_id(&run.context)?;
            prep.cwd = Some(review_cwd(app, id, "ai_review")?);
            prep.extra_dirs.push(scratch_for("cr", id)?);
            if let Some(home) = dirs::home_dir() {
                prep.extra_dirs.push(home.join(".claude/skills/local-review"));
                prep.extra_dirs.push(home.join(".claude/plugins/cache/openai-codex"));
            }
        }
        Some("image_diff_report") => {
            let id = review_id(&run.context)?;
            let dir = image_diff_dir(app, id).canonicalize()?;
            let report = dir.join(report::RAW);
            // The raw report may be rewritten by a previous run's leftovers: start clean.
            let _ = std::fs::remove_file(&report);
            prep.extra_tools.push(format!("Edit(/{})", report.display()));
            prep.system_suffix = format!(
                "\n\n---\n\nApp mode. Review id = {id}. The image diffs are in `{}` (your working directory). \
                 Write the report with the Write tool to `{}` (the only writable path).",
                dir.display(),
                report.display()
            );
            prep.cwd = Some(dir.clone());
            prep.extra_dirs.push(dir);
        }
        Some("issue_plan") => {
            // The plan lands in the notes vault, outside the clone the run starts in: grant exactly that tree.
            let s = routine_settings(&app.state::<Db>().0.lock().unwrap());
            let root = canonical_dir(&s.notes_root)?;
            let plans = canonical_dir(&s.plans_dir)?;
            for dir in [&root, &plans] {
                prep.extra_tools.push(format!("Write(/{}/**)", dir.display()));
                prep.extra_tools.push(format!("Edit(/{}/**)", dir.display()));
            }
            prep.extra_dirs.push(root.clone());
            if !plans.starts_with(&root) {
                prep.extra_dirs.push(plans);
            }
        }
        _ => {}
    }
    Ok(prep)
}

/// A configured directory (`~/` allowed), created when missing, in canonical form so path rules match.
fn canonical_dir(configured: &str) -> AppResult<PathBuf> {
    let dir = PathBuf::from(crate::commands::repo::expand_home(configured));
    std::fs::create_dir_all(&dir)?;
    Ok(dir.canonicalize()?)
}

/// Post-process a run that ended (not on app exit: the row is settled elsewhere).
pub fn finish(app: &AppHandle, routine: &Routine, run: &RoutineRun, status: &str, text: &str, error: Option<&str>, usage: &RunUsage) {
    let succeeded = matches!(status, "ok" | "needs_attention");
    let failure = || error.map(str::to_string).unwrap_or_else(|| format!("the run ended: {status}"));
    match routine.handler.as_deref() {
        Some("overview") => {
            let ids = OVERVIEW_IDS.lock().unwrap().as_mut().and_then(|m| m.remove(&run.id)).unwrap_or_default();
            if succeeded && !text.trim().is_empty() {
                let model = effective_model(run, routine);
                let db = app.state::<Db>();
                let conn = db.0.lock().unwrap();
                let saved = conn.execute(
                    "INSERT INTO board_overview (id, markdown, generated_at, model, card_count, input_tokens, output_tokens, cost_usd, card_ids)
                     VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(id) DO UPDATE SET markdown = excluded.markdown, generated_at = excluded.generated_at,
                       model = excluded.model, card_count = excluded.card_count, input_tokens = excluded.input_tokens,
                       output_tokens = excluded.output_tokens, cost_usd = excluded.cost_usd, card_ids = excluded.card_ids",
                    params![
                        text,
                        Utc::now().to_rfc3339(),
                        (!model.is_empty()).then_some(model),
                        ids.len() as i64,
                        usage.input_tokens,
                        usage.output_tokens,
                        usage.cost_usd,
                        serde_json::to_string(&ids).unwrap_or_else(|_| "[]".into())
                    ],
                );
                if let Err(e) = saved {
                    log::warn!(target: "routines", "run {}: storing the overview failed: {e}", run.id);
                }
            }
        }
        Some("focus") => {
            if let Ok(id) = review_id(&run.context) {
                // `cr focus set` already flipped the status to ready on success; a row still
                // `generating` means the run failed or never submitted a plan.
                let err = (!succeeded).then(failure);
                let _ = finish_generation(&app.state::<Db>().0.lock().unwrap(), id, err);
                let _ = app.emit("focus-updated", FocusUpdated { review_id: id });
            }
        }
        Some("ai_review") => {
            if let Ok(id) = review_id(&run.context) {
                if succeeded {
                    emit_ai_review(app, id, layer(routine), "done", None, Some(text.to_string()));
                } else {
                    emit_ai_review(app, id, layer(routine), "failed", Some(failure()), None);
                }
            }
        }
        Some("image_diff_report") => {
            if let Ok(id) = review_id(&run.context) {
                let outcome = if succeeded {
                    report::finalize(&image_diff_dir(app, id)).map(|_| ()).map_err(|e| e.to_string())
                } else {
                    Err(failure())
                };
                match outcome {
                    Ok(()) => emit_report(app, id, "done", None),
                    Err(e) => emit_report(app, id, "failed", Some(e)),
                }
            }
        }
        _ => {}
    }
}

/// `finish` for a run that never reached `finalize` (a queued run dropped by Stop).
pub fn finish_dropped(app: &AppHandle, run_id: i64) {
    let loaded = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        get_run(&conn, run_id).and_then(|run| super::get(&conn, run.routine_id).map(|r| (run, r)))
    };
    if let Ok((run, routine)) = loaded {
        finish(app, &routine, &run, "killed", "", Some("stopped before it started"), &RunUsage::default());
    }
}

/// Whether an image diff report run is queued or running for the review.
pub fn active_image_diff_report(conn: &rusqlite::Connection, review_id: i64) -> bool {
    conn.query_row(
        "SELECT 1 FROM routine_run r JOIN routine ro ON ro.id = r.routine_id
         WHERE ro.handler = 'image_diff_report' AND r.status IN ('queued', 'running')
           AND json_extract(r.context_json, '$.kind') = 'review'
           AND json_extract(r.context_json, '$.id') = ?1",
        params![review_id.to_string()],
        |_| Ok(()),
    )
    .is_ok()
}

/// The AI review layer running on a review, if any.
pub fn active_ai_review_layer(conn: &rusqlite::Connection, review_id: i64) -> Option<String> {
    conn.query_row(
        "SELECT ro.key FROM routine_run r JOIN routine ro ON ro.id = r.routine_id
         WHERE ro.handler = 'ai_review' AND r.status IN ('queued', 'running')
           AND json_extract(r.context_json, '$.kind') = 'review'
           AND json_extract(r.context_json, '$.id') = ?1",
        params![review_id.to_string()],
        |r| r.get::<_, String>(0),
    )
    .ok()
    .map(|k| k.strip_prefix(AI_REVIEW_PREFIX).unwrap_or(&k).to_string())
}
