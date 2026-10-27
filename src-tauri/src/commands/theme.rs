//! AI-suggested custom themes (Settings → Theme → AI suggest).

use tauri::State;

use crate::ai_usage::RunTag;
use crate::commands::chat::{run_claude, ClaudeRun};
use crate::db::Db;
use crate::error::{AppError, AppResult};

const MODEL: &str = "sonnet";
const SCOPES: [&str; 5] = ["all", "colours", "syntax", "typography", "shape"];

/// The user message of a suggestion run.
fn user_message(current: &serde_json::Value, scope: &str, inspiration: &str, feedback: Option<&str>) -> String {
    let fix = feedback
        .filter(|f| !f.trim().is_empty())
        .map(|f| format!("Fix: your previous suggestion had these problems, correct them:\n{f}\n"))
        .unwrap_or_default();
    format!("Scope: {scope}\nInspiration: {inspiration}\n{fix}Current theme (JSON):\n{current}\n")
}

/// Ask Claude for a custom theme. Returns the model's raw reply; the frontend extracts and
/// validates the JSON (`src/lib/themeSuggest.ts`).
#[tauri::command]
pub async fn theme_suggest(
    current: serde_json::Value,
    scope: String,
    inspiration: String,
    feedback: Option<String>,
    db: State<'_, Db>,
) -> AppResult<String> {
    crate::ai_gate::ensure_enabled()?;
    if crate::tools::claude_path().is_none() {
        return Err(AppError::Other("the `claude` CLI was not found on PATH".into()));
    }
    if !SCOPES.contains(&scope.as_str()) {
        return Err(AppError::Other(format!("unknown theme scope: {scope}")));
    }
    let system = {
        let conn = db.0.lock().unwrap();
        crate::prompts::resolve(&conn, crate::prompts::THEME_SUGGEST)?
    };
    let prompt = user_message(&current, &scope, inspiration.trim(), feedback.as_deref());
    let turn = tauri::async_runtime::spawn_blocking(move || {
        let cwd = std::env::temp_dir().join("codereview_theme_suggest");
        std::fs::create_dir_all(&cwd)?;
        run_claude(ClaudeRun {
            model: MODEL,
            allowed_tools: &[],
            permission_mode: "default",
            system_prompt: Some(&system),
            prompt_file_tag: "theme_suggest",
            cwd: &cwd,
            extra_dirs: &[],
            session: None,
            env: vec![("CLAUDE_SUMMARY_RUNNING".into(), "1".into())],
            extra_args: &["--tools", "", "--effort", "medium"],
            replace_system_prompt: true,
            prompt: &prompt,
            tag: RunTag::new("theme_suggest").variant(scope.as_str()).label(inspiration.trim().to_string()),
        })
    })
    .await
    .map_err(|e| AppError::Other(format!("theme task failed: {e}")))??;
    Ok(turn.text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_carries_scope_inspiration_and_current_theme() {
        let current = serde_json::json!({ "base": "a" });
        let m = user_message(&current, "syntax", "nordic dusk", None);
        assert!(m.starts_with("Scope: syntax\nInspiration: nordic dusk\n"));
        assert!(m.contains("{\"base\":\"a\"}"));
        assert!(!m.contains("Fix:"));
    }

    #[test]
    fn feedback_is_added_only_when_not_blank() {
        let current = serde_json::json!({});
        assert!(user_message(&current, "all", "x", Some("dark: text on surface is 3.1:1")).contains("Fix:"));
        assert!(!user_message(&current, "all", "x", Some("  ")).contains("Fix:"));
    }
}
