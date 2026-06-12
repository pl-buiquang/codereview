use std::collections::HashMap;

use serde_json::json;

use crate::db::models::{Comment, ReviewDetail};

fn short_sha(sha: &Option<String>) -> String {
    sha.as_ref()
        .map(|s| s.chars().take(8).collect())
        .unwrap_or_else(|| "?".into())
}

/// Map root comment id -> its replies, in `created_at` (then `id`) order. The
/// single source of thread structure shared by publish and Markdown/JSON export.
pub fn replies_by_root(comments: &[Comment]) -> HashMap<i64, Vec<&Comment>> {
    let mut map: HashMap<i64, Vec<&Comment>> = HashMap::new();
    for c in comments {
        if let Some(pid) = c.parent_id {
            map.entry(pid).or_default().push(c);
        }
    }
    for replies in map.values_mut() {
        replies.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    }
    map
}

/// Append each reply to `body` as a `> **reply by me:**` blockquote block — the
/// single locked folding format shared by publish and Markdown export. With no
/// replies, returns the trimmed body unchanged.
pub fn fold_replies(body: &str, replies: &[&Comment]) -> String {
    let mut out = body.trim().to_string();
    for r in replies {
        out.push_str("\n\n> **reply by me:**");
        for line in r.body.trim().lines() {
            out.push_str("\n> ");
            out.push_str(line);
        }
    }
    out
}

/// Render a review to deterministic, AI-friendly Markdown: location + diff hunk
/// + comment body for each note, preceded by the verdict and summary.
pub fn render_markdown(detail: &ReviewDetail, repo_label: &str) -> String {
    let r = &detail.review;
    let t = &detail.target;
    let mut out = String::new();

    out.push_str(&format!("# Review: {}\n", t.title));
    out.push_str(&format!(
        "Repo: {repo_label} · Base: {} ({}) → Head: {} ({})\n",
        t.base_ref,
        short_sha(&t.base_sha),
        t.head_ref,
        short_sha(&t.head_sha),
    ));
    if let Some(ev) = &r.event {
        out.push_str(&format!("Verdict: {ev}\n"));
    }
    out.push_str(&format!("Status: {}\n\n", r.status));

    if !r.body.trim().is_empty() {
        out.push_str("## Summary\n\n");
        out.push_str(r.body.trim());
        out.push_str("\n\n");
    }

    if !detail.comments.is_empty() {
        let replies = replies_by_root(&detail.comments);
        out.push_str("## Comments\n\n");
        for c in &detail.comments {
            if c.parent_id.is_some() {
                continue; // replies nest under their root, never top-level
            }
            let folded = fold_replies(&c.body, replies.get(&c.id).map_or(&[][..], Vec::as_slice));
            if c.subject_type == "file" {
                out.push_str(&format!("### {} (whole file)\n\n", c.file_path));
                out.push_str(&folded);
                out.push_str("\n\n");
                continue;
            }
            if c.origin == "file_view" {
                let loc = match c.start_line {
                    Some(start) if start != c.line => {
                        format!("{}:L{}-L{}", c.file_path, start, c.line)
                    }
                    _ => format!("{}:L{}", c.file_path, c.line),
                };
                out.push_str(&format!("### {loc} (file view)\n\n"));
                out.push_str(&folded);
                out.push_str("\n\n");
                continue;
            }
            let loc = match c.start_line {
                Some(start) if start != c.line => {
                    format!("{}:{}-{}", c.file_path, start, c.line)
                }
                _ => format!("{}:{}", c.file_path, c.line),
            };
            out.push_str(&format!("### {} ({})\n\n", loc, c.side));
            if let Some(hunk) = &c.diff_hunk {
                if !hunk.trim().is_empty() {
                    out.push_str("```diff\n");
                    out.push_str(hunk.trim_end());
                    out.push_str("\n```\n\n");
                }
            }
            out.push_str(&folded);
            out.push_str("\n\n");
        }
    }

    out
}

/// Render the same review model to JSON for programmatic consumers.
pub fn render_json(detail: &ReviewDetail, repo_label: &str) -> String {
    let r = &detail.review;
    let t = &detail.target;
    let replies = replies_by_root(&detail.comments);
    let comments: Vec<_> = detail
        .comments
        .iter()
        .filter(|c| c.parent_id.is_none())
        .map(|c| {
            let nested: Vec<_> = replies
                .get(&c.id)
                .map_or(&[][..], Vec::as_slice)
                .iter()
                .map(|r| json!({ "body": r.body, "created_at": r.created_at }))
                .collect();
            json!({
                "file": c.file_path,
                "subject_type": c.subject_type,
                "origin": c.origin,
                "side": c.side,
                "line": c.line,
                "start_line": c.start_line,
                "diff_hunk": c.diff_hunk,
                "body": c.body,
                "replies": nested,
            })
        })
        .collect();

    let value = json!({
        "title": t.title,
        "repo": repo_label,
        "kind": t.kind,
        "github_pr_number": t.github_pr_number,
        "base_ref": t.base_ref,
        "base_sha": t.base_sha,
        "head_ref": t.head_ref,
        "head_sha": t.head_sha,
        "verdict": r.event,
        "status": r.status,
        "summary": r.body,
        "comments": comments,
    });

    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::models::{Comment, Review, Target};

    fn target() -> Target {
        Target {
            id: 1,
            repo_id: 1,
            kind: "github_pr".into(),
            github_pr_number: Some(42),
            title: "Improve thing".into(),
            base_ref: "main".into(),
            head_ref: "feature".into(),
            base_sha: Some("abcdef1234567890".into()),
            head_sha: Some("1234567890abcdef".into()),
            three_dot: true,
            created_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn review() -> Review {
        Review {
            id: 1,
            target_id: 1,
            body: "Looks good overall.".into(),
            event: Some("approve".into()),
            status: "draft".into(),
            published_at: None,
            github_review_id: None,
            last_exported_at: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn comment(line: i64, start_line: Option<i64>) -> Comment {
        Comment {
            id: 1,
            review_id: 1,
            file_path: "src/main.rs".into(),
            subject_type: "line".into(),
            origin: "diff".into(),
            side: "RIGHT".into(),
            line,
            start_line,
            diff_hunk: Some("@@ -1,2 +1,3 @@\n line1\n+line3".into()),
            body: "Consider renaming this.".into(),
            parent_id: None,
            anchored_head_sha: None,
            github_comment_id: None,
            resolved_at: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn detail(comments: Vec<Comment>) -> ReviewDetail {
        ReviewDetail {
            review: review(),
            target: target(),
            repo_path: "/repo".into(),
            remote_owner: Some("owner".into()),
            remote_name: Some("name".into()),
            comments,
            viewed_files: vec![],
        }
    }

    #[test]
    fn short_sha_truncates_to_eight() {
        assert_eq!(short_sha(&Some("1234567890abcdef".into())), "12345678");
    }

    #[test]
    fn short_sha_handles_short_input_and_none() {
        assert_eq!(short_sha(&Some("abc".into())), "abc");
        assert_eq!(short_sha(&None), "?");
    }

    #[test]
    fn markdown_includes_header_verdict_summary_and_comment() {
        let md = render_markdown(&detail(vec![comment(3, None)]), "owner/repo");
        assert!(md.contains("# Review: Improve thing"));
        assert!(md.contains("Repo: owner/repo · Base: main (abcdef12) → Head: feature (12345678)"));
        assert!(md.contains("Verdict: approve"));
        assert!(md.contains("Status: draft"));
        assert!(md.contains("## Summary"));
        assert!(md.contains("Looks good overall."));
        assert!(md.contains("### src/main.rs:3 (RIGHT)"));
        assert!(md.contains("```diff"));
        assert!(md.contains("Consider renaming this."));
    }

    #[test]
    fn markdown_renders_multiline_range_location() {
        let md = render_markdown(&detail(vec![comment(5, Some(3))]), "owner/repo");
        assert!(md.contains("### src/main.rs:3-5 (RIGHT)"), "got: {md}");
    }

    #[test]
    fn markdown_single_line_when_start_equals_line() {
        let md = render_markdown(&detail(vec![comment(5, Some(5))]), "owner/repo");
        assert!(md.contains("### src/main.rs:5 (RIGHT)"));
        assert!(!md.contains("5-5"));
    }

    #[test]
    fn markdown_omits_summary_when_body_blank() {
        let mut d = detail(vec![]);
        d.review.body = "   ".into();
        let md = render_markdown(&d, "owner/repo");
        assert!(!md.contains("## Summary"));
        assert!(!md.contains("## Comments"));
    }

    #[test]
    fn markdown_omits_verdict_when_none() {
        let mut d = detail(vec![]);
        d.review.event = None;
        let md = render_markdown(&d, "owner/repo");
        assert!(!md.contains("Verdict:"));
    }

    #[test]
    fn json_is_valid_and_carries_fields() {
        let json = render_json(&detail(vec![comment(5, Some(3))]), "owner/repo");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["title"], "Improve thing");
        assert_eq!(v["repo"], "owner/repo");
        assert_eq!(v["kind"], "github_pr");
        assert_eq!(v["github_pr_number"], 42);
        assert_eq!(v["verdict"], "approve");
        assert_eq!(v["status"], "draft");
        assert_eq!(v["comments"][0]["file"], "src/main.rs");
        assert_eq!(v["comments"][0]["line"], 5);
        assert_eq!(v["comments"][0]["start_line"], 3);
        assert_eq!(v["comments"][0]["side"], "RIGHT");
    }

    fn file_comment(body: &str) -> Comment {
        Comment {
            subject_type: "file".into(),
            line: 0,
            start_line: None,
            diff_hunk: None,
            body: body.into(),
            ..comment(0, None)
        }
    }

    #[test]
    fn markdown_renders_file_comment_as_whole_file() {
        let md = render_markdown(&detail(vec![file_comment("Module needs a doc comment.")]), "owner/repo");
        assert!(md.contains("### src/main.rs (whole file)"), "got: {md}");
        assert!(md.contains("Module needs a doc comment."));
        // No line range and no diff hunk for file-level comments.
        assert!(!md.contains("src/main.rs:0"));
        assert!(!md.contains("```diff"));
    }

    #[test]
    fn json_carries_subject_type() {
        let json = render_json(&detail(vec![file_comment("note")]), "owner/repo");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["comments"][0]["subject_type"], "file");
    }

    fn file_view_comment(line: i64, start_line: Option<i64>, body: &str) -> Comment {
        Comment {
            origin: "file_view".into(),
            line,
            start_line,
            diff_hunk: None,
            body: body.into(),
            ..comment(line, start_line)
        }
    }

    #[test]
    fn markdown_renders_file_view_comment_with_line_and_no_diff() {
        let md = render_markdown(
            &detail(vec![file_view_comment(7, None, "pane note")]),
            "owner/repo",
        );
        assert!(md.contains("### src/main.rs:L7 (file view)"), "got: {md}");
        assert!(md.contains("pane note"));
        assert!(!md.contains("```diff"));
    }

    #[test]
    fn markdown_file_view_comment_renders_range() {
        let md = render_markdown(
            &detail(vec![file_view_comment(9, Some(7), "range note")]),
            "owner/repo",
        );
        assert!(md.contains("### src/main.rs:L7-L9 (file view)"), "got: {md}");
    }

    #[test]
    fn json_carries_origin() {
        let json = render_json(&detail(vec![file_view_comment(7, None, "note")]), "owner/repo");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["comments"][0]["origin"], "file_view");
    }

    #[test]
    fn json_null_start_line_preserved() {
        let json = render_json(&detail(vec![comment(5, None)]), "owner/repo");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v["comments"][0]["start_line"].is_null());
    }

    // ---- threaded replies (spec 11) ----

    fn reply(id: i64, parent_id: i64, body: &str) -> Comment {
        Comment {
            id,
            parent_id: Some(parent_id),
            body: body.into(),
            created_at: format!("2026-01-01T00:00:0{id}Z"),
            ..comment(5, None)
        }
    }

    #[test]
    fn fold_replies_quotes_multiline_bodies() {
        let r1 = reply(2, 1, "first line\nsecond line");
        let r2 = reply(3, 1, "another");
        let folded = fold_replies("root body", &[&r1, &r2]);
        assert!(folded.starts_with("root body"));
        assert!(folded.contains("\n\n> **reply by me:**\n> first line\n> second line"));
        assert!(folded.contains("\n\n> **reply by me:**\n> another"));
        // No replies returns the trimmed body unchanged.
        assert_eq!(fold_replies("  spaced  ", &[]), "spaced");
    }

    #[test]
    fn markdown_nests_replies_under_root() {
        let md = render_markdown(
            &detail(vec![comment(3, None), reply(2, 1, "a quoted reply")]),
            "owner/repo",
        );
        // One root section; the reply text appears quoted, not as its own heading.
        assert_eq!(md.matches("### ").count(), 1, "got: {md}");
        assert!(md.contains("> **reply by me:**"));
        assert!(md.contains("> a quoted reply"));
    }

    #[test]
    fn json_groups_replies_under_root() {
        let json = render_json(
            &detail(vec![comment(5, None), reply(2, 1, "nested reply")]),
            "owner/repo",
        );
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let comments = v["comments"].as_array().unwrap();
        assert_eq!(comments.len(), 1, "reply excluded from top-level");
        assert_eq!(v["comments"][0]["replies"][0]["body"], "nested reply");
        assert!(v["comments"][0]["replies"][0]["created_at"].is_string());
    }

    #[test]
    fn json_root_without_replies_has_empty_array() {
        let json = render_json(&detail(vec![comment(5, None)]), "owner/repo");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["comments"][0]["replies"].as_array().unwrap().len(), 0);
    }
}
