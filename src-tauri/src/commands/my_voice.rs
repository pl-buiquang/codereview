//! "My voice": rewrite an AI comment with the user's own `my-voice` Claude skill into a new
//! draft at the same anchor. The skill does not ship with the app; the action is offered only
//! when `~/.claude/skills/my-voice/SKILL.md` exists ([`skill_path`]).

use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use crate::ai::profiles::{self, Feature};
use crate::ai::{Access, OneShot, SessionMode, SystemPromptMode, ToolPolicy};
use crate::commands::review::{add_sibling_comment_impl, ensure_draft, get_comment, is_ai_tag};
use crate::db::models::Comment;
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::run_usage::RunTag;

const SKILL: &str = "my-voice";
const MODEL: &str = "sonnet";
const TOOLS: [&str; 3] = ["Skill", "Read", "Glob"];

/// The installed skill directory, if its `SKILL.md` exists.
pub fn skill_path() -> Option<PathBuf> {
    let dir = dirs::home_dir()?.join(".claude/skills").join(SKILL);
    dir.join("SKILL.md").is_file().then_some(dir)
}

#[derive(Debug, Serialize)]
pub struct MyVoiceResult {
    /// The new draft, untagged so it publishes with the review.
    pub comment: Comment,
    /// The skill's "Changes" list (what it dropped, hedged or reworded); may be empty.
    pub changes: String,
}

fn location(c: &Comment) -> String {
    if c.subject_type == "file" {
        return format!("{} (whole file)", c.file_path);
    }
    match c.start_line {
        Some(s) if s != c.line => format!("{}:{}-{} ({})", c.file_path, s, c.line, c.side),
        _ => format!("{}:{} ({})", c.file_path, c.line, c.side),
    }
}

fn prompt(c: &Comment) -> String {
    format!(
        "/{SKILL}\n\n\
         Medium: GitHub PR inline review comment on `{loc}`. Recipient: the PR author. Language: English.\n\
         This run is non-interactive: never ask a question, decide yourself.\n\
         The draft was written by an AI reviewer. Drop its bold `**[severity · source · title]**` \
         header if it has one; keep code identifiers, suggestion blocks and links as they are.\n\
         Output exactly: the rewrite inside <rewrite>...</rewrite>, then the Changes list. Nothing else, \
         and do not write any file.\n\n\
         Draft:\n{body}\n",
        loc = location(c),
        body = c.body.trim(),
    )
}

/// Split the skill output into (rewrite, changes). Without a `<rewrite>` block the whole
/// reply is the rewrite.
fn parse(text: &str) -> (String, String) {
    if let Some(start) = text.find("<rewrite>") {
        let rest = &text[start + "<rewrite>".len()..];
        if let Some(end) = rest.find("</rewrite>") {
            let changes = rest[end + "</rewrite>".len()..].trim();
            let changes = changes
                .strip_prefix("**Changes**")
                .or_else(|| changes.strip_prefix("Changes:"))
                .or_else(|| changes.strip_prefix("## Changes"))
                .unwrap_or(changes)
                .trim();
            return (rest[..end].trim().to_string(), changes.to_string());
        }
    }
    (text.trim().to_string(), String::new())
}

fn request<'a>(
    prompt: &'a str,
    cwd: &'a std::path::Path,
    skill_dir: &'a std::path::Path,
    model: &'a str,
    review_id: i64,
) -> OneShot<'a> {
    OneShot {
        model,
        system_prompt: None,
        system_prompt_mode: SystemPromptMode::Append,
        file_tag: "my_voice",
        prompt,
        cwd,
        extra_dirs: vec![skill_dir],
        tools: ToolPolicy::Allowlist(TOOLS.iter().map(|t| t.to_string()).collect()),
        access: Access::Default,
        session: SessionMode::Ephemeral,
        max_turns: None,
        effort: Some("low"),
        env: Vec::new(),
        extra_args: Vec::new(),
        usage_tag: RunTag::new("my_voice").review(review_id).harness("claude"),
    }
}

/// Rewrite the AI comment `comment_id` with the `my-voice` skill and add the result as a new
/// untagged draft at the same anchor. The original is left as it is.
#[tauri::command]
pub async fn my_voice_rewrite(comment_id: i64, db: State<'_, Db>) -> AppResult<MyVoiceResult> {
    crate::ai_gate::ensure_enabled()?;
    let skill_dir = skill_path().ok_or_else(|| {
        AppError::Other(format!(
            "the {SKILL} skill is not installed in ~/.claude/skills"
        ))
    })?;
    let harness = crate::ai::harness_by_id("claude")
        .ok_or_else(|| AppError::Other("Claude Code harness missing".into()))?;
    if let Some(reason) = harness.unavailable_reason() {
        return Err(AppError::Other(reason));
    }
    let (source, model) = {
        let conn = db.0.lock().unwrap();
        let c = get_comment(&conn, comment_id)?;
        ensure_draft(&conn, c.review_id)?;
        if !is_ai_tag(c.tag.as_deref()) {
            return Err(AppError::Other(
                "My voice works on AI-tagged comments only".into(),
            ));
        }
        // Skills are Claude Code's: use the Chat profile's model only when it is a Claude one.
        let resolved = profiles::resolve_feature(&conn, Feature::Chat);
        let model = if resolved.harness.id() == "claude" {
            resolved.model_or_builtin(MODEL)
        } else {
            MODEL.to_string()
        };
        (c, model)
    };
    let text = prompt(&source);
    let review_id = source.review_id;
    let turn = tauri::async_runtime::spawn_blocking(move || {
        let cwd = std::env::temp_dir().join("codereview_my_voice");
        std::fs::create_dir_all(&cwd)?;
        harness.one_shot(&request(&text, &cwd, &skill_dir, &model, review_id))
    })
    .await
    .map_err(|e| AppError::Other(format!("my-voice task failed: {e}")))??;
    let (body, changes) = parse(&turn.text);
    if body.is_empty() {
        return Err(AppError::Other(
            "the my-voice skill returned nothing".into(),
        ));
    }
    let conn = db.0.lock().unwrap();
    let comment = add_sibling_comment_impl(&conn, comment_id, &body)?;
    Ok(MyVoiceResult { comment, changes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rewrite_and_changes() {
        let (r, c) =
            parse("<rewrite>\nnit: rename this\n</rewrite>\n\n**Changes**\n- dropped the header");
        assert_eq!(r, "nit: rename this");
        assert_eq!(c, "- dropped the header");
    }

    #[test]
    fn without_markers_the_whole_reply_is_the_rewrite() {
        assert_eq!(parse("  just text \n"), ("just text".into(), String::new()));
        assert_eq!(
            parse("<rewrite>unclosed"),
            ("<rewrite>unclosed".into(), String::new())
        );
    }

    #[test]
    fn run_argv() {
        use crate::test_support::{argv, strs};
        let cwd = std::path::Path::new("/tmp/codereview_my_voice");
        let skill = std::path::Path::new("/home/me/.claude/skills/my-voice");
        let req = request("p", cwd, skill, "sonnet", 7);
        let cmd = crate::ai::claude::command(&req, false, None);
        let args = argv(&cmd);
        for want in strs(&[
            "--allowedTools",
            "Skill",
            "Read",
            "Glob",
            "--no-session-persistence",
        ]) {
            assert!(args.contains(&want), "{want} in {args:?}");
        }
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--add-dir" && w[1] == skill.to_str().unwrap()));
    }
}
