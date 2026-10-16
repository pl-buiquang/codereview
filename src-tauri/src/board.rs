//! Kanban board pure logic: lanes, the pending-threads badge, issue<->branch
//! matching and reference parsing. GitHub reads go through the mirror; the DB
//! orchestration lives in `commands/board.rs`.

use serde::Serialize;

use crate::db::models::BoardCardRow;
use crate::error::{AppError, AppResult};
use crate::gh::{self, PrThread};

// ---------------------------------------------------------------------------
// Lanes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    Backlog,
    InProgress,
    Submitted,
    Reviews,
    Done,
}

impl Lane {
    pub fn as_str(self) -> &'static str {
        match self {
            Lane::Backlog => "backlog",
            Lane::InProgress => "in_progress",
            Lane::Submitted => "submitted",
            Lane::Reviews => "reviews",
            Lane::Done => "done",
        }
    }
}

/// Computed lane for a card, or `None` when it shouldn't appear on the board
/// at all (archived, discarded, a closed issue, or an issue hidden behind an
/// open linked PR). `has_open_linked_pr` is only meaningful for issue cards.
pub fn lane_for(card: &BoardCardRow, has_open_linked_pr: bool) -> Option<Lane> {
    if card.archived_at.is_some() || card.discarded_at.is_some() {
        return None;
    }
    if card.typ == "issue" {
        if card.state.as_deref() != Some("open") {
            return None;
        }
        if has_open_linked_pr {
            return None;
        }
        return Some(Lane::Backlog);
    }
    // PR card.
    match card.state.as_deref() {
        Some("merged") | Some("closed") => Some(Lane::Done),
        _ => {
            if card.is_mine {
                if card.is_draft {
                    Some(Lane::InProgress)
                } else {
                    Some(Lane::Submitted)
                }
            } else {
                Some(Lane::Reviews)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Review-thread comment badge
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub enum ThreadMode {
    /// My own PR: the ball is in my court whenever the last word isn't mine.
    Mine,
    /// Someone else's PR I'm reviewing: only threads I started count, so the
    /// badge tracks "they replied, I haven't looked yet" rather than every
    /// unresolved conversation on the PR.
    Reviewing,
}

/// Count of unresolved review threads whose last comment isn't the viewer's —
/// never counting a resolved thread, even if the last comment isn't theirs.
pub fn pending_threads(threads: &[PrThread], viewer: &str, mode: ThreadMode) -> u32 {
    threads
        .iter()
        .filter(|t| {
            if t.is_resolved {
                return false;
            }
            let Some(first) = t.comments.first() else {
                return false;
            };
            let Some(last) = t.comments.last() else {
                return false;
            };
            fn login_of(c: &gh::PrThreadComment) -> Option<&str> {
                c.author.as_ref().and_then(|a| a.login.as_deref())
            }
            if login_of(last) == Some(viewer) {
                return false;
            }
            match mode {
                ThreadMode::Mine => true,
                ThreadMode::Reviewing => login_of(first) == Some(viewer),
            }
        })
        .count() as u32
}

// ---------------------------------------------------------------------------
// Branch <-> issue auto-link
// ---------------------------------------------------------------------------

/// Whether `head_ref` names `issue_number` by the `issue-<n>` convention
/// (e.g. `pbq/issue-123`, `issue-123-typo-fix`), bounded by a `/` or the start
/// of the string on the left and a non-digit or the end of the string on the
/// right, so `issue-123` never matches `issue-1234`.
pub fn branch_matches_issue(head_ref: &str, issue_number: i64) -> bool {
    let needle = format!("issue-{issue_number}");
    let bytes = head_ref.as_bytes();
    let mut start = 0;
    while let Some(pos) = head_ref[start..].find(&needle) {
        let idx = start + pos;
        let before_ok = idx == 0 || bytes[idx - 1] == b'/';
        let after = idx + needle.len();
        let after_ok = after == bytes.len() || !bytes[after].is_ascii_digit();
        if before_ok && after_ok {
            return true;
        }
        start = idx + 1;
    }
    false
}

/// The issue number named by `issue-<n>` in a branch (the `--here` CLI
/// resolution path), or `None` if the branch doesn't follow the convention.
pub fn issue_number_from_branch(head_ref: &str) -> Option<i64> {
    let marker = "issue-";
    let idx = head_ref.find(marker)?;
    let before_ok = idx == 0 || head_ref.as_bytes()[idx - 1] == b'/';
    if !before_ok {
        return None;
    }
    let rest = &head_ref[idx + marker.len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

// ---------------------------------------------------------------------------
// Reference parsing (`cr board` ref args, "+ Add by URL")
// ---------------------------------------------------------------------------

/// Parse a GitHub issue/PR reference, either a full URL
/// (`https://github.com/owner/repo/issues/42` or `.../pull/42`) or the short
/// form `owner/repo#42`. Returns `(owner, name, number)` — the union lookup
/// (`mirror::fetch::resolve`) tells issue from PR, so the caller never has to.
pub fn parse_ref(s: &str) -> AppResult<(String, String, i64)> {
    let s = s.trim();
    for prefix in ["https://github.com/", "http://github.com/"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            let parts: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
            if parts.len() >= 4 && (parts[2] == "issues" || parts[2] == "pull") {
                let owner = parts[0].to_string();
                let name = parts[1].to_string();
                let number: i64 = parts[3]
                    .split(['#', '?'])
                    .next()
                    .unwrap_or("")
                    .parse()
                    .map_err(|_| AppError::Other(format!("invalid GitHub URL: {s}")))?;
                return Ok((owner, name, number));
            }
            return Err(AppError::Other(format!("invalid GitHub URL: {s}")));
        }
    }
    if let Some((repo_part, num_part)) = s.split_once('#') {
        if let Some((owner, name)) = repo_part.split_once('/') {
            let number: i64 = num_part
                .parse()
                .map_err(|_| AppError::Other(format!("invalid reference: {s}")))?;
            return Ok((owner.to_string(), name.to_string(), number));
        }
    }
    Err(AppError::Other(format!("could not parse reference: {s}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(typ: &str, state: Option<&str>, is_draft: bool, is_mine: bool) -> BoardCardRow {
        BoardCardRow {
            id: "X".into(),
            typ: typ.into(),
            repo: "acme/widget".into(),
            number: 1,
            url: "u".into(),
            title: "t".into(),
            author_login: None,
            state: state.map(str::to_string),
            is_draft,
            review_decision: None,
            has_reviews: false,
            head_ref: None,
            is_mine,
            pending_threads: None,
            updated_at: "now".into(),
            closed_at: None,
            tracked_at: "now".into(),
            discarded_at: None,
            archived_at: None,
            last_refreshed: None,
            ci_status: None,
            mergeable: None,
            merge_state_status: None,
            stack_number: None,
            stack_position: None,
            viewer_can_update_branch: false,
            auto_merge: None,
        }
    }

    #[test]
    fn open_issue_with_no_linked_pr_is_backlog() {
        let c = card("issue", Some("open"), false, false);
        assert_eq!(lane_for(&c, false), Some(Lane::Backlog));
    }

    #[test]
    fn open_issue_with_open_linked_pr_is_hidden() {
        let c = card("issue", Some("open"), false, false);
        assert_eq!(lane_for(&c, true), None);
    }

    #[test]
    fn closed_issue_leaves_the_board() {
        let c = card("issue", Some("closed"), false, false);
        assert_eq!(lane_for(&c, false), None);
    }

    #[test]
    fn my_draft_pr_is_in_progress() {
        let c = card("pr", Some("open"), true, true);
        assert_eq!(lane_for(&c, false), Some(Lane::InProgress));
    }

    #[test]
    fn my_ready_pr_is_submitted() {
        let c = card("pr", Some("open"), false, true);
        assert_eq!(lane_for(&c, false), Some(Lane::Submitted));
    }

    #[test]
    fn others_open_pr_is_reviews() {
        let c = card("pr", Some("open"), false, false);
        assert_eq!(lane_for(&c, false), Some(Lane::Reviews));
    }

    #[test]
    fn merged_or_closed_pr_is_done_regardless_of_owner() {
        assert_eq!(lane_for(&card("pr", Some("merged"), false, true), false), Some(Lane::Done));
        assert_eq!(lane_for(&card("pr", Some("closed"), false, false), false), Some(Lane::Done));
    }

    #[test]
    fn discarded_or_archived_cards_are_hidden() {
        let mut c = card("pr", Some("open"), false, false);
        c.discarded_at = Some("now".into());
        assert_eq!(lane_for(&c, false), None);
        c.discarded_at = None;
        c.archived_at = Some("now".into());
        assert_eq!(lane_for(&c, false), None);
    }

    fn thread(resolved: bool, authors: &[&str]) -> PrThread {
        PrThread {
            id: "T".into(),
            is_resolved: resolved,
            is_outdated: false,
            is_collapsed: false,
            path: Some("a.rs".into()),
            line: Some(1),
            start_line: None,
            original_line: Some(1),
            diff_side: Some("RIGHT".into()),
            start_diff_side: None,
            subject_type: Some("LINE".into()),
            comments: authors
                .iter()
                .map(|login| gh::PrThreadComment {
                    id: "C".into(),
                    database_id: Some(1),
                    author: Some(gh::PrActor {
                        login: Some(login.to_string()),
                        avatar_url: None,
                    }),
                    body: "b".into(),
                    created_at: "now".into(),
                    url: "u".into(),
                    diff_hunk: None,
                    outdated: false,
                })
                .collect(),
        }
    }

    #[test]
    fn mine_counts_unresolved_threads_not_last_authored_by_viewer() {
        let threads = vec![
            thread(false, &["reviewer"]),            // last=reviewer -> counts
            thread(false, &["reviewer", "me"]),       // last=me -> doesn't count
            thread(true, &["reviewer"]),              // resolved -> never counts
        ];
        assert_eq!(pending_threads(&threads, "me", ThreadMode::Mine), 1);
    }

    #[test]
    fn reviewing_only_counts_threads_i_started() {
        let threads = vec![
            thread(false, &["me", "author"]),   // I started it, author replied -> counts
            thread(false, &["someone_else", "author"]), // not mine -> doesn't count
            thread(false, &["me", "me"]),        // I started it but I spoke last -> doesn't count
        ];
        assert_eq!(pending_threads(&threads, "me", ThreadMode::Reviewing), 1);
    }

    #[test]
    fn branch_matches_issue_respects_boundaries() {
        assert!(branch_matches_issue("pbq/issue-123", 123));
        assert!(branch_matches_issue("issue-123-typo-fix", 123));
        assert!(!branch_matches_issue("issue-1234", 123));
        assert!(!branch_matches_issue("notissue-123", 123));
        assert!(!branch_matches_issue("pbq/issue-123", 12));
    }

    #[test]
    fn issue_number_from_branch_extracts_the_number() {
        assert_eq!(issue_number_from_branch("pbq/issue-123"), Some(123));
        assert_eq!(issue_number_from_branch("issue-45-fix"), Some(45));
        assert_eq!(issue_number_from_branch("feature/foo"), None);
        assert_eq!(issue_number_from_branch("notissue-123"), None);
    }

    #[test]
    fn parse_ref_accepts_urls_and_short_form() {
        assert_eq!(
            parse_ref("https://github.com/acme/widget/issues/42").unwrap(),
            ("acme".into(), "widget".into(), 42)
        );
        assert_eq!(
            parse_ref("https://github.com/acme/widget/pull/7").unwrap(),
            ("acme".into(), "widget".into(), 7)
        );
        assert_eq!(
            parse_ref("https://github.com/acme/widget/pull/7/files").unwrap(),
            ("acme".into(), "widget".into(), 7)
        );
        assert_eq!(parse_ref("acme/widget#42").unwrap(), ("acme".into(), "widget".into(), 42));
        assert!(parse_ref("not a ref").is_err());
        assert!(parse_ref("https://example.com/acme/widget/issues/42").is_err());
    }
}
