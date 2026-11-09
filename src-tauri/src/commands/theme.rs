//! AI-suggested custom themes (Settings → Theme → AI suggest).

use tauri::State;

use crate::ai_usage::RunTag;
use crate::ai::{self, Access, OneShot, SessionMode, SystemPrompt, SystemPromptMode, ToolPolicy};
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

fn suggest_request<'a>(
    system: &'a str,
    cwd: &'a std::path::Path,
    prompt: &'a str,
    scope: &'a str,
    inspiration: &str,
) -> OneShot<'a> {
    OneShot {
        model: MODEL,
        system_prompt: Some(SystemPrompt::Text(system)),
        system_prompt_mode: SystemPromptMode::Replace,
        file_tag: "theme_suggest",
        prompt,
        cwd,
        extra_dirs: Vec::new(),
        tools: ToolPolicy::None,
        access: Access::Default,
        session: SessionMode::Ephemeral,
        max_turns: None,
        effort: Some("medium"),
        env: vec![("CLAUDE_SUMMARY_RUNNING".into(), "1".into())],
        extra_args: Vec::new(),
        usage_tag: RunTag::new("theme_suggest").variant(scope).label(inspiration.to_string()),
    }
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
    if !ai::harness_for().available() {
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
        ai::harness_for().one_shot(&suggest_request(&system, &cwd, &prompt, &scope, inspiration.trim()))
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

    #[test]
    fn suggest_run_argv_and_env() {
        use crate::test_support::{argv, envs, strs};
        let cwd = std::path::Path::new("/tmp/codereview_theme_suggest");
        let req = suggest_request("sys", cwd, "prompt", "syntax", "nordic");
        let cmd = crate::ai::claude::command(&req, false, Some(std::path::Path::new("/tmp/codereview_theme_suggest.txt")));
        assert_eq!(
            argv(&cmd),
            strs(&[
                "-p", "--output-format", "json", "--strict-mcp-config", "--permission-mode", "default",
                "--add-dir", "/tmp/codereview_theme_suggest", "--model", "sonnet", "--tools", "", "--effort",
                "medium", "--no-session-persistence", "--system-prompt-file", "/tmp/codereview_theme_suggest.txt",
            ])
        );
        assert_eq!(envs(&cmd), vec![("CLAUDE_SUMMARY_RUNNING".to_string(), "1".to_string())]);
        assert_eq!(cmd.get_current_dir(), Some(cwd));
    }
}
