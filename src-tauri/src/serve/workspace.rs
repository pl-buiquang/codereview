//! Workspace resolution: from `{repo: owner/name, pr?, branch?, kind}` to a directory on this
//! machine. The repo must be registered in the daemon's codereview DB (the `cr repo` registry);
//! nothing is cloned. Worktrees are created or reused through `worktree.rs`, so the existing GC
//! rules apply (the janitor treats a directory with a live harness process as in use).

use std::path::{Path, PathBuf};

use base64::Engine as _;
use rusqlite::{params, Connection, OptionalExtension};

use super::request::{Attachment, WorkspaceKind, WorkspaceSpec};
use super::{ApiError, ErrorCode};
use crate::db::models::Target;
use crate::error::AppResult;
use crate::gh::{run_gh, GhRepo};
use crate::worktree;

/// The head branch and sha of a PR, asked of GitHub through `gh`.
pub type PrHead<'a> = &'a dyn Fn(&Path, i64) -> AppResult<(String, String)>;

pub fn gh_pr_head(repo: &Path, number: i64) -> AppResult<(String, String)> {
    let out = run_gh(&GhRepo::Local(repo.to_path_buf()), &["pr", "view", &number.to_string(), "--json", "headRefName,headRefOid"])?;
    let v: serde_json::Value = serde_json::from_str(&out)?;
    let field = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
    match (field("headRefName"), field("headRefOid")) {
        (Some(b), Some(s)) => Ok((b, s)),
        _ => Err(crate::error::AppError::Gh(format!("unexpected `gh pr view` output for #{number}"))),
    }
}

/// A branch name that is safe to hand to git: `git check-ref-format --branch` accepts it and it
/// cannot be read as an option (leading `-`) or climb out of a directory (`..`).
pub fn validate_branch(repo: &Path, branch: &str) -> Result<(), ApiError> {
    let bad = |why: &str| ApiError::bad_request(format!("invalid branch `{branch}`: {why}"));
    if branch.is_empty() || branch.starts_with('-') {
        return Err(bad("empty or starts with `-`"));
    }
    if branch.contains("..") || branch.contains("@{") || branch.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(bad("forbidden characters"));
    }
    let ok = std::process::Command::new("git")
        .current_dir(repo)
        .args(["check-ref-format", "--branch", branch])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        Err(bad("not a valid branch name"))
    }
}

/// An option-like value (`--dangerous-flag`) must never reach a harness' argv as a model or tool name.
pub fn validate_arg_value(what: &str, value: &str) -> Result<(), ApiError> {
    if value.starts_with('-') || value.chars().any(char::is_control) {
        return Err(ApiError::bad_request(format!("invalid {what} `{value}`")));
    }
    Ok(())
}

/// Resolve `spec` with the real worktree base and `gh`.
pub fn resolve(conn: &Connection, spec: &WorkspaceSpec, allow_primary: bool) -> Result<PathBuf, ApiError> {
    resolve_in(conn, spec, &worktree::worktrees_base(), &gh_pr_head, allow_primary)
}

/// `allow_primary`: whether a branch or PR worktree request may land in the registered main clone
/// (its branch is checked out there). Only runs that cannot write may share the user's checkout.
fn is_primary(repo: &Path, path: &Path) -> bool {
    match (repo.canonicalize(), path.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => repo == path,
    }
}

pub fn resolve_in(conn: &Connection, spec: &WorkspaceSpec, base: &Path, pr_head: PrHead, allow_primary: bool) -> Result<PathBuf, ApiError> {
    resolve_locked(conn, spec, base, pr_head, allow_primary, None)
}

/// [`resolve_in`] holding `lock` only around the steps that create or switch a worktree. Asking
/// GitHub for a PR's head happens before the lock is taken, so a slow `gh` stalls only its own request.
pub fn resolve_locked(
    conn: &Connection,
    spec: &WorkspaceSpec,
    base: &Path,
    pr_head: PrHead,
    allow_primary: bool,
    lock: Option<&std::sync::Mutex<()>>,
) -> Result<PathBuf, ApiError> {
    let lock = || lock.map(|l| l.lock().unwrap_or_else(|e| e.into_inner()));
    let (owner, name) = spec
        .repo
        .split_once('/')
        .filter(|(o, n)| !o.is_empty() && !n.is_empty() && !n.contains('/'))
        .ok_or_else(|| ApiError::bad_request(format!("`{}` is not owner/name", spec.repo)))?;
    let local: Option<Option<String>> = conn
        .query_row(
            "SELECT local_path FROM repository
             WHERE remote_owner = ?1 COLLATE NOCASE AND remote_name = ?2 COLLATE NOCASE
             ORDER BY local_path IS NULL, id LIMIT 1",
            params![owner, name],
            |r| r.get(0),
        )
        .optional()?;
    let Some(local) = local else {
        return Err(ApiError::new(ErrorCode::UnknownRepo, format!("{} is not registered on this host (cr repo add <path>)", spec.repo)));
    };
    let repo = match local.map(PathBuf::from) {
        Some(p) if p.is_dir() => p,
        _ => return Err(ApiError::new(ErrorCode::NoWorktree, format!("{} is registered without a local clone on this host", spec.repo))),
    };
    let no_wt = |e: crate::error::AppError| ApiError::new(ErrorCode::NoWorktree, e.to_string());
    let not_primary = |path: PathBuf| -> Result<PathBuf, ApiError> {
        if !allow_primary && is_primary(&repo, &path) {
            return Err(ApiError::new(
                ErrorCode::NoWorktree,
                format!("that branch is checked out in the main clone of {}; a run that can write needs its own worktree", spec.repo),
            ));
        }
        Ok(path)
    };
    if let Some(branch) = spec.branch.as_deref().filter(|b| !b.is_empty()) {
        validate_branch(&repo, branch)?;
    }
    match spec.kind {
        WorkspaceKind::MainClone => Ok(repo),
        WorkspaceKind::BranchWorktree => {
            let branch = spec.branch.as_deref().filter(|b| !b.is_empty()).ok_or_else(|| ApiError::bad_request("branch-worktree needs `branch`"))?;
            let _guard = lock();
            let path = worktree::ensure_branch_worktree_in(&repo, &target("local", None, branch), None, base).map_err(no_wt)?;
            not_primary(path)
        }
        WorkspaceKind::PrWorktree => {
            let number = spec.pr.ok_or_else(|| ApiError::bad_request("pr-worktree needs `pr`"))?;
            // The PR's real head branch always comes from GitHub: a client-supplied hint is only
            // trusted when it matches, so it cannot point a run at an unrelated worktree.
            // Not trusted offline either: with `gh` unreachable the request fails rather than
            // guessing which worktree the hint should map to.
            let (head_ref, head_sha) = pr_head(&repo, number).map_err(|e| {
                ApiError::new(
                    ErrorCode::NoWorktree,
                    format!("could not verify the head of PR #{number} with GitHub ({e}); the branch hint is not used without it, retry when `gh` is reachable"),
                )
            })?;
            validate_branch(&repo, &head_ref)?;
            let _guard = lock();
            if let Some(hint) = spec.branch.as_deref().filter(|b| !b.is_empty()) {
                if hint == head_ref {
                    let existing = worktree::list_worktrees(&repo).map_err(no_wt)?;
                    if let Some(wt) = existing.iter().find(|w| !w.is_prunable && w.branch.as_deref() == Some(hint)) {
                        return not_primary(PathBuf::from(&wt.path));
                    }
                }
            }
            let path = worktree::ensure_branch_worktree_in(&repo, &target("github_pr", Some(number), &head_ref), Some(&head_sha), base).map_err(no_wt)?;
            not_primary(path)
        }
    }
}

fn target(kind: &str, pr: Option<i64>, head_ref: &str) -> Target {
    Target {
        id: 0,
        repo_id: 0,
        kind: kind.into(),
        github_pr_number: pr,
        gh_pr_id: None,
        title: String::new(),
        base_ref: String::new(),
        head_ref: head_ref.into(),
        three_dot: true,
        created_at: String::new(),
    }
}

/// `<runs base>/scratch/<run id>`: attachments of a run, removed with its cleanup. Run ids are
/// unique across harnesses, so one directory serves them all. `runs_base` is the run root
/// (`serve_runs`).
pub fn scratch_dir(runs_base: &Path, run_id: i64) -> PathBuf {
    runs_base.join("scratch").join(run_id.to_string())
}

/// Largest attachment (decoded) and the total the daemon accepts for one run.
pub const MAX_ATTACHMENT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_ATTACHMENTS_TOTAL: usize = 96 * 1024 * 1024;

/// Check names and sizes and decode, before anything touches the disk.
pub fn decode_attachments(items: &[Attachment]) -> Result<Vec<(String, Vec<u8>)>, ApiError> {
    let mut out = Vec::new();
    let mut total = 0usize;
    for a in items {
        let ok = !a.name.is_empty()
            && a.name != "."
            && a.name != ".."
            && !a.name.contains(['/', '\\', '\0']);
        if !ok {
            return Err(ApiError::bad_request(format!("attachment name `{}` must be a plain file name", a.name)));
        }
        if out.iter().any(|(n, _): &(String, Vec<u8>)| n == &a.name) {
            return Err(ApiError::bad_request(format!("attachment `{}` is listed twice", a.name)));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(a.content_base64.as_bytes())
            .map_err(|e| ApiError::bad_request(format!("attachment `{}` is not valid base64: {e}", a.name)))?;
        total += bytes.len();
        if bytes.len() > MAX_ATTACHMENT_BYTES || total > MAX_ATTACHMENTS_TOTAL {
            return Err(ApiError::bad_request("attachments are too large"));
        }
        out.push((a.name.clone(), bytes));
    }
    Ok(out)
}

/// Write the decoded attachments into the run's scratch directory (created only when there are some).
pub fn write_attachments(scratch: &Path, files: &[(String, Vec<u8>)]) -> std::io::Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(scratch)?;
    for (name, bytes) in files {
        std::fs::write(scratch.join(name), bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let st = Command::new("git").current_dir(dir).args(args).output().unwrap();
        assert!(st.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&st.stderr));
    }

    /// A repo with a `feat` branch and one commit, remote `acme/widgets`, registered in a fresh app DB.
    struct Fixture {
        _dir: tempfile::TempDir,
        repo: PathBuf,
        base: PathBuf,
        conn: Connection,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("widgets");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "t@example.com"]);
        git(&repo, &["config", "user.name", "t"]);
        git(&repo, &["remote", "add", "origin", "https://github.com/acme/widgets.git"]);
        std::fs::write(repo.join("a.txt"), "a").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        git(&repo, &["branch", "feat"]);
        let conn = crate::db::open(&dir.path().join("app.db")).unwrap();
        crate::commands::repo::add_repository_impl(&conn, repo.to_str().unwrap()).unwrap();
        Fixture { base: dir.path().join("wt"), _dir: dir, repo, conn }
    }

    fn spec(repo: &str, kind: WorkspaceKind, pr: Option<i64>, branch: Option<&str>) -> WorkspaceSpec {
        WorkspaceSpec { repo: repo.into(), pr, branch: branch.map(Into::into), kind }
    }

    fn no_gh(_: &Path, _: i64) -> AppResult<(String, String)> {
        panic!("gh must not be asked")
    }

    #[test]
    fn github_is_asked_outside_the_workspace_lock_and_an_offline_gh_is_a_clear_error() {
        let f = fixture();
        let lock = std::sync::Mutex::new(());
        let offline = |_: &Path, _: i64| -> AppResult<(String, String)> {
            assert!(lock.try_lock().is_ok(), "gh must be asked without the lock");
            Err(crate::error::AppError::Gh("timed out".into()))
        };
        let s = spec("acme/widgets", WorkspaceKind::PrWorktree, Some(7), Some("feat"));
        let err = resolve_locked(&f.conn, &s, &f.base, &offline, true, Some(&lock)).unwrap_err();
        assert_eq!(err.code, ErrorCode::NoWorktree);
        assert!(err.message.contains("could not verify the head of PR #7") && err.message.contains("timed out"), "{}", err.message);
        // reachable gh, existing worktree for the verified head: found, lock free during the ask
        let wt = f.base.join("feat-wt");
        git(&f.repo, &["worktree", "add", wt.to_str().unwrap(), "feat"]);
        let online = |_: &Path, _: i64| -> AppResult<(String, String)> {
            assert!(lock.try_lock().is_ok(), "gh must be asked without the lock");
            Ok(("feat".into(), "0".repeat(40)))
        };
        let got = resolve_locked(&f.conn, &s, &f.base, &online, false, Some(&lock)).unwrap();
        assert_eq!(got.canonicalize().unwrap(), wt.canonicalize().unwrap());
    }

    #[test]
    fn unknown_repo_is_a_clear_error() {
        let f = fixture();
        let err = resolve_in(&f.conn, &spec("acme/other", WorkspaceKind::MainClone, None, None), &f.base, &no_gh, true).unwrap_err();
        assert_eq!(err.code, ErrorCode::UnknownRepo);
        assert!(err.message.contains("acme/other"));
        let err = resolve_in(&f.conn, &spec("nonsense", WorkspaceKind::MainClone, None, None), &f.base, &no_gh, true).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadRequest);
    }

    #[test]
    fn main_clone_is_the_registered_path_and_the_lookup_ignores_case() {
        let f = fixture();
        let got = resolve_in(&f.conn, &spec("ACME/Widgets", WorkspaceKind::MainClone, None, None), &f.base, &no_gh, true).unwrap();
        assert_eq!(got, f.repo);
    }

    #[test]
    fn a_registered_repo_without_a_clone_is_no_worktree() {
        let f = fixture();
        f.conn.execute("UPDATE repository SET local_path = NULL", []).unwrap();
        let err = resolve_in(&f.conn, &spec("acme/widgets", WorkspaceKind::MainClone, None, None), &f.base, &no_gh, true).unwrap_err();
        assert_eq!(err.code, ErrorCode::NoWorktree);
    }

    #[test]
    fn branch_worktree_is_created_once_then_reused() {
        let f = fixture();
        let s = spec("acme/widgets", WorkspaceKind::BranchWorktree, None, Some("feat"));
        let first = resolve_in(&f.conn, &s, &f.base, &no_gh, true).unwrap();
        assert!(first.starts_with(&f.base) && first.join("a.txt").exists(), "{first:?}");
        let second = resolve_in(&f.conn, &s, &f.base, &no_gh, true).unwrap();
        assert_eq!(first, second);
        let missing = spec("acme/widgets", WorkspaceKind::BranchWorktree, None, Some("nope"));
        assert_eq!(resolve_in(&f.conn, &missing, &f.base, &no_gh, true).unwrap_err().code, ErrorCode::NoWorktree);
        assert_eq!(
            resolve_in(&f.conn, &spec("acme/widgets", WorkspaceKind::BranchWorktree, None, None), &f.base, &no_gh, true).unwrap_err().code,
            ErrorCode::BadRequest
        );
    }

    #[test]
    fn pr_worktree_is_reused_when_the_hint_matches_what_gh_says() {
        let f = fixture();
        let wt = f.base.join("manual");
        std::fs::create_dir_all(&f.base).unwrap();
        git(&f.repo, &["worktree", "add", "-q", wt.to_str().unwrap(), "feat"]);
        let asked = std::cell::Cell::new(0);
        let head = |_: &Path, n: i64| -> AppResult<(String, String)> {
            assert_eq!(n, 7);
            asked.set(asked.get() + 1);
            Ok(("feat".into(), "0".repeat(40)))
        };
        let hinted = spec("acme/widgets", WorkspaceKind::PrWorktree, Some(7), Some("feat"));
        let a = resolve_in(&f.conn, &hinted, &f.base, &head, true).unwrap();
        assert_eq!(a.canonicalize().unwrap(), wt.canonicalize().unwrap());
        // without a hint the head branch comes from gh as well, and the same worktree is found
        let b = resolve_in(&f.conn, &spec("acme/widgets", WorkspaceKind::PrWorktree, Some(7), None), &f.base, &head, true).unwrap();
        assert_eq!(b.canonicalize().unwrap(), wt.canonicalize().unwrap());
        assert_eq!(asked.get(), 2);
        assert_eq!(
            resolve_in(&f.conn, &spec("acme/widgets", WorkspaceKind::PrWorktree, None, None), &f.base, &no_gh, true).unwrap_err().code,
            ErrorCode::BadRequest
        );
    }

    #[test]
    fn a_wrong_pr_hint_is_not_trusted() {
        let f = fixture();
        let wt = f.base.join("manual");
        std::fs::create_dir_all(&f.base).unwrap();
        git(&f.repo, &["worktree", "add", "-q", wt.to_str().unwrap(), "feat"]);
        // the client claims PR 7 is on `feat`, but gh says it is on a branch that does not exist
        let head = |_: &Path, _: i64| -> AppResult<(String, String)> { Ok(("other".into(), "0".repeat(40))) };
        let hinted = spec("acme/widgets", WorkspaceKind::PrWorktree, Some(7), Some("feat"));
        let err = resolve_in(&f.conn, &hinted, &f.base, &head, true);
        match err {
            Ok(p) => assert_ne!(p.canonicalize().unwrap(), wt.canonicalize().unwrap()),
            Err(e) => assert_eq!(e.code, ErrorCode::NoWorktree),
        }
    }

    #[test]
    fn the_main_clones_branch_is_not_shared_with_runs_that_can_write() {
        let f = fixture();
        let s = spec("acme/widgets", WorkspaceKind::BranchWorktree, None, Some("main"));
        let err = resolve_in(&f.conn, &s, &f.base, &no_gh, false).unwrap_err();
        assert_eq!(err.code, ErrorCode::NoWorktree);
        // a read-only run may look at the user's checkout
        let ok = resolve_in(&f.conn, &s, &f.base, &no_gh, true).unwrap();
        assert_eq!(ok.canonicalize().unwrap(), f.repo.canonicalize().unwrap());
        // the PR flavour with a matching hint goes through the same guard
        let head = |_: &Path, _: i64| -> AppResult<(String, String)> { Ok(("main".into(), "0".repeat(40))) };
        let pr = spec("acme/widgets", WorkspaceKind::PrWorktree, Some(1), Some("main"));
        assert_eq!(resolve_in(&f.conn, &pr, &f.base, &head, false).unwrap_err().code, ErrorCode::NoWorktree);
    }

    #[test]
    fn branch_names_are_validated_before_git_sees_them() {
        let f = fixture();
        for bad in ["--upload-pack=x", "-b", "a..b", "feat/../x", "a b", "x@{1}", "~x", "a:b", "x.lock"] {
            assert!(validate_branch(&f.repo, bad).is_err(), "{bad}");
            let s = spec("acme/widgets", WorkspaceKind::BranchWorktree, None, Some(bad));
            assert_eq!(resolve_in(&f.conn, &s, &f.base, &no_gh, true).unwrap_err().code, ErrorCode::BadRequest, "{bad}");
        }
        for ok in ["feat", "pbq/issue-3", "release-1.2"] {
            validate_branch(&f.repo, ok).unwrap();
        }
        assert!(validate_arg_value("model", "--evil").is_err());
        assert!(validate_arg_value("model", "openrouter/x").is_ok());
    }

    #[test]
    fn attachments_are_validated_then_written_to_scratch() {
        let enc = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        let ok = decode_attachments(&[Attachment { name: "a.txt".into(), content_base64: enc(b"hello") }]).unwrap();
        assert_eq!(ok, vec![("a.txt".to_string(), b"hello".to_vec())]);
        for bad in ["../x", "a/b", "", "..", "a\\b"] {
            assert!(decode_attachments(&[Attachment { name: bad.into(), content_base64: enc(b"x") }]).is_err(), "{bad}");
        }
        assert!(decode_attachments(&[Attachment { name: "a".into(), content_base64: "!!".into() }]).is_err());
        let dup = Attachment { name: "a".into(), content_base64: enc(b"x") };
        assert!(decode_attachments(&[dup.clone(), dup]).is_err());

        let dir = tempfile::tempdir().unwrap();
        let scratch = scratch_dir(&dir.path().join("runs"), 5);
        assert_eq!(scratch, dir.path().join("runs").join("scratch").join("5"));
        write_attachments(&scratch, &ok).unwrap();
        assert_eq!(std::fs::read(scratch.join("a.txt")).unwrap(), b"hello");
        write_attachments(&dir.path().join("none"), &[]).unwrap();
        assert!(!dir.path().join("none").exists());
    }
}
