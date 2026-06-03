use serde_json::json;

use crate::db::models::ReviewDetail;

fn short_sha(sha: &Option<String>) -> String {
    sha.as_ref()
        .map(|s| s.chars().take(8).collect())
        .unwrap_or_else(|| "?".into())
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
        out.push_str("## Comments\n\n");
        for c in &detail.comments {
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
            out.push_str(c.body.trim());
            out.push_str("\n\n");
        }
    }

    out
}

/// Render the same review model to JSON for programmatic consumers.
pub fn render_json(detail: &ReviewDetail, repo_label: &str) -> String {
    let r = &detail.review;
    let t = &detail.target;
    let comments: Vec<_> = detail
        .comments
        .iter()
        .map(|c| {
            json!({
                "file": c.file_path,
                "side": c.side,
                "line": c.line,
                "start_line": c.start_line,
                "diff_hunk": c.diff_hunk,
                "body": c.body,
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
