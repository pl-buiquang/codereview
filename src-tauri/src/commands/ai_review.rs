//! Headless `/local-review` runs launched from the review actions menu. They run as the built-in
//! `ai_review_*` routines (`routines::handlers`), which enforce one layer per review at a time and
//! announce progress with the `ai-review-updated` event.

use tauri::{AppHandle, State};

use crate::commands::routines::{builtin, launch};
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::routines::context::ContextRef;
use crate::routines::handlers;

pub(crate) const TOOLS: &[&str] = &[
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

fn routine_key(layer: &str) -> AppResult<String> {
    match layer {
        "explain" | "critique" | "codex" => Ok(format!("ai_review_{layer}")),
        other => Err(AppError::Other(format!("unknown AI review layer `{other}`"))),
    }
}

/// Start a background `/local-review` run (`layer`: explain | critique | codex).
#[tauri::command]
pub fn run_ai_review(
    app: AppHandle,
    review_id: i64,
    layer: String,
    model: Option<String>,
    db: State<Db>,
) -> AppResult<()> {
    let routine = builtin(&db, &routine_key(&layer)?)?;
    let ctx = ContextRef { kind: "review".into(), id: review_id.to_string(), base_paths: vec![] };
    launch(&app, &routine, &ctx, "", model.as_deref().unwrap_or(""))?;
    Ok(())
}

/// The layer currently running for the review, if any.
#[tauri::command]
pub fn ai_review_status(review_id: i64, db: State<Db>) -> Option<String> {
    handlers::active_ai_review_layer(&db.0.lock().unwrap(), review_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_maps_to_routine_key() {
        assert_eq!(routine_key("critique").unwrap(), "ai_review_critique");
        assert!(routine_key("nope").is_err());
    }
}
