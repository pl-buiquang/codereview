//! Headless `/local-review` runs launched from the review actions menu. One `claude -p` run per
//! review at a time; progress is announced with the `ai-review-updated` event.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{Emitter, Manager, State};

use crate::commands::chat::{resolve_model, run_claude, ClaudeRun};
use crate::commands::focus::{child_path, review_local_path};
use crate::commands::review::ensure_draft;
use crate::db::{Db, DbPath};
use crate::error::{AppError, AppResult};

/// Layers currently running, keyed by review id.
#[derive(Default)]
pub struct AiReviewJobs(Mutex<HashMap<i64, String>>);

const TOOLS: &[&str] = &[
    "Agent",
    "Skill",
    "Read",
    "Glob",
    "Grep",
    "Write",
    "Edit",
    "Bash(cr:*)",
    "Bash(git:*)",
    "Bash(jq:*)",
    "Bash(node:*)",
    "Bash(ls:*)",
    "Bash(mkdir:*)",
    "Bash(test:*)",
    "Bash(gh pr view:*)",
    "Bash(gh pr diff:*)",
    "Bash(gh issue view:*)",
    "Bash(gh api:*)",
];

#[derive(Clone, Serialize)]
struct AiReviewUpdated {
    review_id: i64,
    layer: String,
    status: &'static str,
    error: Option<String>,
    summary: Option<String>,
}

fn prompt_for(layer: &str, review_id: i64) -> AppResult<String> {
    let flag = match layer {
        "explain" => "",
        "critique" => " --critique",
        "codex" => " --codex",
        other => return Err(AppError::Other(format!("unknown AI review layer `{other}`"))),
    };
    Ok(format!("/local-review {review_id}{flag}"))
}

fn system_prompt(review_id: i64) -> String {
    format!(
        "Headless app mode. Review id = {review_id} already exists: do not create or open reviews. \
         The `cr` CLI on PATH already points at the app's database. Nobody can answer questions, so \
         never use AskUserQuestion; pick sensible defaults. This session ends when you stop, so it \
         cannot wait for background notifications: run the Codex companion in the foreground, never as \
         a background Bash call. Finish with a summary of at most 5 lines."
    )
}

fn claim(jobs: &AiReviewJobs, review_id: i64, layer: &str) -> AppResult<()> {
    let mut map = jobs.0.lock().unwrap();
    if let Some(running) = map.get(&review_id) {
        return Err(AppError::Other(format!("an AI review ({running}) is already running for this review")));
    }
    map.insert(review_id, layer.to_string());
    Ok(())
}

fn run(layer: &str, review_id: i64, model: &str, local_path: Option<String>, db_path: &Path) -> AppResult<String> {
    let temp;
    let cwd = match &local_path {
        Some(p) if Path::new(p).exists() => Path::new(p),
        _ => {
            temp = std::env::temp_dir().join(format!("codereview_ai_review_{review_id}"));
            std::fs::create_dir_all(&temp)?;
            temp.as_path()
        }
    };
    let mut env = vec![("CODEREVIEW_DB".to_string(), db_path.as_os_str().to_owned())];
    if let Some(path) = child_path() {
        env.push(("PATH".to_string(), path));
    }
    let scratch = std::env::temp_dir().canonicalize()?.join(format!("cr-{review_id}"));
    std::fs::create_dir_all(&scratch)?;
    let mut extra_dirs = vec![scratch];
    if let Some(home) = dirs::home_dir() {
        extra_dirs.push(home.join(".claude/skills/local-review"));
        extra_dirs.push(home.join(".claude/plugins/cache/openai-codex"));
    }
    let extra: Vec<&Path> = extra_dirs.iter().map(|p| p.as_path()).collect();
    let prompt = prompt_for(layer, review_id)?;
    let sys = system_prompt(review_id);
    run_claude(ClaudeRun {
        model,
        allowed_tools: TOOLS,
        // Explicit so the user's own settings (e.g. `defaultMode: plan`) never leak in.
        permission_mode: "acceptEdits",
        system_prompt: Some(&sys),
        prompt_file_tag: &format!("ai_review_{review_id}"),
        cwd,
        extra_dirs: &extra,
        session: None,
        env,
        extra_args: &[],
        replace_system_prompt: false,
        prompt: &prompt,
    })
    .map(|t| t.text)
}

/// Start a background `/local-review` run (`layer`: explain | critique | codex).
#[tauri::command]
pub fn run_ai_review(
    app: tauri::AppHandle,
    review_id: i64,
    layer: String,
    model: Option<String>,
    db: State<Db>,
    db_path: State<DbPath>,
    jobs: State<AiReviewJobs>,
) -> AppResult<()> {
    prompt_for(&layer, review_id)?;
    if crate::tools::claude_path().is_none() {
        return Err(AppError::Other("the `claude` CLI was not found on PATH".into()));
    }
    let model = resolve_model(model);
    let local_path = {
        let conn = db.0.lock().unwrap();
        ensure_draft(&conn, review_id)?;
        review_local_path(&conn, review_id)?
    };
    claim(&jobs, review_id, &layer)?;
    let db_path = db_path.0.clone();
    let _ = app.emit(
        "ai-review-updated",
        AiReviewUpdated { review_id, layer: layer.clone(), status: "running", error: None, summary: None },
    );
    tauri::async_runtime::spawn_blocking(move || {
        let result = run(&layer, review_id, &model, local_path, &db_path);
        app.state::<AiReviewJobs>().0.lock().unwrap().remove(&review_id);
        let (status, error, summary) = match result {
            Ok(text) => ("done", None, Some(text)),
            Err(e) => ("failed", Some(e.to_string()), None),
        };
        let _ = app.emit(
            "ai-review-updated",
            AiReviewUpdated { review_id, layer, status, error, summary },
        );
    });
    Ok(())
}

/// The layer currently running for the review, if any.
#[tauri::command]
pub fn ai_review_status(review_id: i64, jobs: State<AiReviewJobs>) -> Option<String> {
    jobs.0.lock().unwrap().get(&review_id).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_per_layer() {
        assert_eq!(prompt_for("explain", 7).unwrap(), "/local-review 7");
        assert_eq!(prompt_for("critique", 7).unwrap(), "/local-review 7 --critique");
        assert_eq!(prompt_for("codex", 7).unwrap(), "/local-review 7 --codex");
        assert!(prompt_for("nope", 7).is_err());
    }

    #[test]
    fn second_run_for_same_review_is_rejected() {
        let jobs = AiReviewJobs::default();
        claim(&jobs, 1, "codex").unwrap();
        let err = claim(&jobs, 1, "critique").unwrap_err();
        assert!(err.to_string().contains("codex"));
        claim(&jobs, 2, "critique").unwrap();
    }
}
