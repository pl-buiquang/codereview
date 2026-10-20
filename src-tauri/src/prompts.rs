use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::error::{AppError, AppResult};

pub struct PromptDef {
    pub key: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub placeholders: &'static [&'static str],
    pub default: &'static str,
}

pub const CHAT_SYSTEM: &str = "chat.system";
pub const FOCUS_SYSTEM: &str = "focus.system";
pub const BOARD_OVERVIEW: &str = "board.overview";
pub const SESSION_LIVE_SUMMARY: &str = "session.live_summary";

const CHAT_DEFAULT: &str = r#"You are a code review assistant for: {{title}}

## Context
- Repository: {{repo}}
{{pr_line}}

## Changed files
{{diff_summary}}

## Capabilities
You have full read/write access to the repository, checked out on branch `{{head_ref}}`.
Code is checked out at: {{worktree_path}}
- Use Bash to run git commands (git log, git show, git blame, git diff, git commit, etc.)
- Use Read/Grep/Glob to explore the codebase
- Use Edit/Write to make code changes
- Commits you make land on `{{head_ref}}` directly (this worktree is a real branch checkout,
  not detached). Only run `git push` when the user explicitly asks you to.

## Guidelines
- Focus on understanding the code changes and their implications
- When asked about specific lines, read the actual file for full context
- Use git blame/log to understand the history of changed code when relevant
- Reference files by their repo-relative path
- Be concise but thorough
- Don't commit or push unless the user asks you to
{{comments}}
## Review operations (cr CLI)
You can read and modify this review using the `cr` command-line tool.
The current review ID is: {{review_id}}

Reading:
  cr show {{review_id}} --json             # full review detail with all comments
  cr diff {{review_id}}                    # the diff being reviewed

Adding comments:
  cr comment add {{review_id}} --file <path> --line <N> --body "..." [--side LEFT|RIGHT]
  cr comment add-file {{review_id}} --file <path> --body "..."   # file-level comment

Modifying comments:
  cr comment update <COMMENT_ID> --body "new text"
  cr comment resolve <COMMENT_ID>
  cr comment unresolve <COMMENT_ID>
  cr comment delete <COMMENT_ID>"#;

const FOCUS_DEFAULT: &str = include_str!("../../skills/local-review/focus.md");

const BOARD_OVERVIEW_DEFAULT: &str = r#"You add judgment on top of a rule-based digest of a developer's PR/issue board. The user
ALREADY sees, computed by rules: review requests waiting on them, approved PRs to merge,
failing CI, conflicts, behind-base, changes requested / unresolved threads, stale items,
per-repo lane counts, issues whose PRs merged, PRs without an issue. Never restate those.

Snapshot: one line per open item, grouped by repo. Fields: lane
(backlog|in_progress|submitted|reviews), mine, by=<author>, draft, ci, review, merge,
threads, upd/tracked (days), issue/prs (links), stack=<id>.<position>, auto_merge,
sess (agent sessions), note (user notes), tags.

Read titles, notes, tags and links together and output only:
## Focus
One or two sentences: the theme worth concentrating on now and why.
## Insights
At most 5 bullets, ≤20 words each, each citing items. Pick only what is notable:
- Clusters: items across repos that are one effort and should move together; likely duplicates.
- Sequencing: what unblocks what (stack order, issue→PR chains, backend before frontend).
- Risk: titles/notes hinting at risky changes (migrations, auth, data, big refactors).
- Contradictions: notes or tags that disagree with state (e.g. "waiting on X" but X merged,
  "urgent" tag on a stale item).
- Batching: reviews or chores that fit one sitting.
If nothing goes beyond the digest, output only "Nothing beyond the digest."

Rules: use only the snapshot, never invent facts. Refer to items exactly as the repo header
says (e.g. cardiologs-front#123) so they become links. Be terse."#;

const SESSION_LIVE_SUMMARY_DEFAULT: &str = r#"You summarize a Claude Code session that is still running. You never continue the conversation or answer its questions; you only describe where it stands. The very first line of your output must be `Title: <session title>`: at most 70 characters, imperative mood, no trailing period. Then a blank line, then terse markdown bullets under only these sections: Goal, Progress, Current state (what is in flight or blocked), Next. Omit empty sections. Output only the title line and the markdown."#;

pub const PROMPTS: &[PromptDef] = &[
    PromptDef {
        key: SESSION_LIVE_SUMMARY,
        label: "Live session summary",
        description: "Instructions for the Sessions view's Generate summary button. Used as the system prompt when summarizing an extract of the transcript, and as the message when forking a warm session.",
        placeholders: &[],
        default: SESSION_LIVE_SUMMARY_DEFAULT,
    },
    PromptDef {
        key: BOARD_OVERVIEW,
        label: "Board AI insights",
        description: "System prompt for the board's AI insights, shown above the rule-based digest. The compact board snapshot is sent as the user message.",
        placeholders: &[],
        default: BOARD_OVERVIEW_DEFAULT,
    },
    PromptDef {
        key: FOCUS_SYSTEM,
        label: "Focus mode",
        description: "System prompt for focus generation (hunk triage). An app-mode suffix with the review id and the `cr focus set` instructions is always appended. Overrides apply in this app only, not to the `/local-review --focus` skill.",
        placeholders: &[],
        default: FOCUS_DEFAULT,
    },
    PromptDef {
        key: CHAT_SYSTEM,
        label: "Review chat",
        description: "System prompt sent on the first turn of a review chat. Edits apply to new chats; clear a chat to pick them up.",
        placeholders: &[
            "title",
            "repo",
            "pr_line",
            "diff_summary",
            "head_ref",
            "worktree_path",
            "comments",
            "review_id",
        ],
        default: CHAT_DEFAULT,
    },
];

fn def(key: &str) -> AppResult<&'static PromptDef> {
    PROMPTS
        .iter()
        .find(|p| p.key == key)
        .ok_or_else(|| AppError::Other(format!("unknown prompt: {key}")))
}

pub fn resolve(conn: &Connection, key: &str) -> AppResult<String> {
    let default = def(key)?.default;
    let custom: Option<String> = conn
        .query_row(
            "SELECT body FROM prompt_override WHERE key = ?1",
            params![key],
            |r| r.get(0),
        )
        .optional()?;
    Ok(custom.unwrap_or_else(|| default.to_string()))
}

/// Single pass, so substituted values containing `{{…}}` are never re-expanded.
pub fn render(tpl: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(tpl.len());
    let mut rest = tpl;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                let name = after[..end].trim();
                match vars.iter().find(|(k, _)| *k == name) {
                    Some((_, v)) => out.push_str(v),
                    None => out.push_str(&rest[start..start + 2 + end + 2]),
                }
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

#[derive(Serialize)]
pub struct PromptInfo {
    pub key: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub placeholders: &'static [&'static str],
    pub default: &'static str,
    #[serde(rename = "override")]
    pub custom: Option<String>,
}

pub fn list(conn: &Connection) -> AppResult<Vec<PromptInfo>> {
    PROMPTS
        .iter()
        .map(|p| {
            let custom: Option<String> = conn
                .query_row(
                    "SELECT body FROM prompt_override WHERE key = ?1",
                    params![p.key],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(PromptInfo {
                key: p.key,
                label: p.label,
                description: p.description,
                placeholders: p.placeholders,
                default: p.default,
                custom,
            })
        })
        .collect()
}

pub fn set(conn: &Connection, key: &str, body: &str) -> AppResult<()> {
    let d = def(key)?;
    if body == d.default {
        return reset(conn, key);
    }
    conn.execute(
        "INSERT INTO prompt_override (key, body, updated_at) VALUES (?1, ?2, datetime('now'))
         ON CONFLICT(key) DO UPDATE SET body = excluded.body, updated_at = excluded.updated_at",
        params![key, body],
    )?;
    Ok(())
}

pub fn reset(conn: &Connection, key: &str) -> AppResult<()> {
    def(key)?;
    conn.execute("DELETE FROM prompt_override WHERE key = ?1", params![key])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_is_single_pass_and_keeps_unknown() {
        let out = render("a {{x}} b {{y}} {{x}}", &[("x", "{{y}}")]);
        assert_eq!(out, "a {{y}} b {{y}} {{y}}");
    }
}
