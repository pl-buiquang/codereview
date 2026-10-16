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

const FOCUS_DEFAULT: &str = include_str!("../../skills/focus-review/SKILL.md");

const BOARD_OVERVIEW_DEFAULT: &str = r#"You are a triage assistant for a developer's personal PR/issue board. You get a compact
snapshot of every open tracked item (one line each, grouped by repo). Fields:
lane (backlog|in_progress|submitted|reviews), mine = authored by the user, by=<author>,
draft, ci, review (decision), merge (mergeStateStatus), threads (unresolved review threads),
upd/tracked = days since last GitHub update / since tracked, issue/prs = links,
sess = linked agent sessions, note = user notes, tags = user tags.

Rules: use only the snapshot; never invent facts.
- Others' PRs (lane=reviews): review=APPROVED needs nothing from the user; REVIEW_REQUIRED or
  CHANGES_REQUESTED means it is waiting on the user's review.
- The user's own drafts (mine draft) are work in progress: never urgent, never in Do next for
  CI/conflicts; mention them only under Stale when upd ≥14d.
- Refer to items as repo#N (short repo name) so they become links.
- Be terse: bullets, max ~12 words each. Skip empty sections.

Output Markdown with exactly these sections:
## TL;DR
2–3 sentences: overall state and the single most important thing to do now.
## Do next
Ordered list (max 7) of concrete actions, highest leverage first. Each: action verb + item
+ why. Prioritize: unblocking others (review requests waiting on the user) > own PRs that are
approved but not merged / behind / conflicting > red CI on own PRs > changes requested >
new work.
## Urgent / blocked
Items with failing CI, conflicts, changes requested, or reviewers waiting >2d.
## Stale
Items with no update ≥7d (PRs) or ≥21d (issues); suggest nudge, rebase, close, or archive.
## Waiting on others
Own PRs pending review/CI where the user has nothing to do.
## By project
One line per repo: counts by lane + a one-phrase theme of what's in flight.
## Hygiene
Board cleanup: issues whose PR merged, PRs with no linked issue, duplicates, items to archive."#;

pub const PROMPTS: &[PromptDef] = &[
    PromptDef {
        key: BOARD_OVERVIEW,
        label: "Board AI overview",
        description: "System prompt for the board's AI Overview. The compact board snapshot is sent as the user message.",
        placeholders: &[],
        default: BOARD_OVERVIEW_DEFAULT,
    },
    PromptDef {
        key: FOCUS_SYSTEM,
        label: "Focus mode",
        description: "System prompt for focus generation (hunk triage). An app-mode suffix with the review id and the `cr focus set` instructions is always appended. Overrides apply in this app only, not to the /focus-review skill.",
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
