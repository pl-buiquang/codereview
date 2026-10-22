//! Mutating git operations for the Graph tab (spec 24). Everything runs in one checkout.
//!
//! Refusals the UI can act on (force delete, switch worktree, force-with-lease…) are
//! recognised from git's stderr and tagged as `[kind] message`, see [`classify`].

use std::path::Path;

use crate::error::{AppError, AppResult};
use crate::git::{run_git, run_git_env, validate_commit_ref};

use super::wip::ensure_inside_checkout;
use super::{StashEntry, WipKind};

const NO_PROMPT_ENV: [(&str, &str); 2] = [("GIT_TERMINAL_PROMPT", "0"), ("GCM_INTERACTIVE", "never")];

fn classify(stderr: &str) -> Option<&'static str> {
    let s = stderr.to_lowercase();
    if s.contains("not fully merged") {
        Some("not_fully_merged")
    } else if s.contains("already used by worktree") || s.contains("already checked out at") {
        Some("checked_out_elsewhere")
    } else if s.contains("would be overwritten")
        || s.contains("commit your changes or stash")
        || s.contains("local changes")
    {
        Some("dirty_tree")
    } else if s.contains("[rejected]") || s.contains("failed to push some refs") {
        Some("push_rejected")
    } else if s.contains("not possible to fast-forward") || s.contains("diverging branches") {
        Some("not_fast_forward")
    } else {
        None
    }
}

fn tag_refusal(err: AppError) -> AppError {
    match err {
        AppError::Git(msg) => match classify(&msg) {
            Some(kind) => AppError::Git(format!("[{kind}] {msg}")),
            None => AppError::Git(msg),
        },
        other => other,
    }
}

fn run(checkout: &Path, args: &[&str]) -> AppResult<String> {
    run_git(checkout, args).map_err(tag_refusal)
}

pub(crate) fn run_network(checkout: &Path, args: &[&str]) -> AppResult<String> {
    run_git_env(checkout, args, &NO_PROMPT_ENV).map_err(tag_refusal)
}

fn validate_branch_name(name: &str) -> AppResult<()> {
    validate_commit_ref(name)?;
    run_git(checkout_dummy(), &["check-ref-format", "--branch", name])
        .map(|_| ())
        .map_err(|_| AppError::Git(format!("invalid branch name: {name}")))
}

fn validate_tag_name(name: &str) -> AppResult<()> {
    validate_commit_ref(name)?;
    run_git(checkout_dummy(), &["check-ref-format", &format!("refs/tags/{name}")])
        .map(|_| ())
        .map_err(|_| AppError::Git(format!("invalid tag name: {name}")))
}

/// `check-ref-format` needs no repository; any existing directory works as cwd.
fn checkout_dummy() -> &'static Path {
    Path::new(".")
}

pub fn checkout_branch(checkout: &Path, name: &str) -> AppResult<()> {
    validate_commit_ref(name)?;
    run(checkout, &["switch", name]).map(|_| ())
}

/// `remote_ref` is the short remote-tracking name, e.g. `origin/feature`.
pub fn checkout_remote_branch(checkout: &Path, remote_ref: &str) -> AppResult<()> {
    validate_commit_ref(remote_ref)?;
    run(checkout, &["switch", "--track", remote_ref]).map(|_| ())
}

pub fn checkout_detached(checkout: &Path, rev: &str) -> AppResult<()> {
    let sha = crate::git::rev_parse(checkout, rev)?;
    run(checkout, &["switch", "--detach", &sha]).map(|_| ())
}

pub fn create_branch(checkout: &Path, name: &str, at: &str, switch: bool) -> AppResult<()> {
    validate_branch_name(name)?;
    let sha = crate::git::rev_parse(checkout, at)?;
    if switch {
        run(checkout, &["switch", "-c", name, &sha]).map(|_| ())
    } else {
        run(checkout, &["branch", name, &sha]).map(|_| ())
    }
}

pub fn delete_branch(checkout: &Path, name: &str, force: bool) -> AppResult<()> {
    validate_commit_ref(name)?;
    run(checkout, &["branch", if force { "-D" } else { "-d" }, "--", name]).map(|_| ())
}

pub fn rename_branch(checkout: &Path, old: &str, new: &str) -> AppResult<()> {
    validate_commit_ref(old)?;
    validate_branch_name(new)?;
    run(checkout, &["branch", "-m", old, new]).map(|_| ())
}

/// A non-empty `message` makes the tag annotated. `push` is the only remote write the graph allows.
pub fn create_tag(
    checkout: &Path,
    name: &str,
    at: &str,
    message: Option<&str>,
    push: bool,
) -> AppResult<()> {
    validate_tag_name(name)?;
    let sha = crate::git::rev_parse(checkout, at)?;
    match message.map(str::trim).filter(|m| !m.is_empty()) {
        Some(m) => run(checkout, &["tag", "-a", name, &sha, "-m", m])?,
        None => run(checkout, &["tag", name, &sha])?,
    };
    if push {
        run_network(checkout, &["push", "origin", &format!("refs/tags/{name}")]).map_err(|e| {
            AppError::Git(format!("tag {name} was created locally but the push failed: {e}"))
        })?;
    }
    Ok(())
}

pub fn delete_tag(checkout: &Path, name: &str) -> AppResult<()> {
    validate_commit_ref(name)?;
    run(checkout, &["tag", "-d", name]).map(|_| ())
}

fn check_paths(paths: &[String]) -> AppResult<()> {
    paths.iter().try_for_each(|p| ensure_inside_checkout(p))
}

/// Empty `paths` means everything.
pub fn stage(checkout: &Path, paths: &[String]) -> AppResult<()> {
    check_paths(paths)?;
    let mut args = vec!["add", "-A", "--"];
    if paths.is_empty() {
        args.push(".");
    }
    args.extend(paths.iter().map(String::as_str));
    run(checkout, &args).map(|_| ())
}

/// Empty `paths` means everything.
pub fn unstage(checkout: &Path, paths: &[String]) -> AppResult<()> {
    check_paths(paths)?;
    let has_head = run_git(checkout, &["rev-parse", "--verify", "-q", "HEAD"]).is_ok();
    let mut args = if has_head {
        vec!["restore", "--staged", "--"]
    } else {
        vec!["rm", "--cached", "-r", "-q", "--"]
    };
    if paths.is_empty() {
        args.push(".");
    }
    args.extend(paths.iter().map(String::as_str));
    run(checkout, &args).map(|_| ())
}

pub fn discard(checkout: &Path, paths: &[String], kind: WipKind) -> AppResult<()> {
    if paths.is_empty() {
        return Ok(());
    }
    check_paths(paths)?;
    let mut args = match kind {
        WipKind::Unstaged => vec!["restore", "--"],
        WipKind::Untracked => vec!["clean", "-f", "--"],
        WipKind::Staged => return Err(AppError::Git("unstage the file before discarding it".into())),
    };
    args.extend(paths.iter().map(String::as_str));
    run(checkout, &args).map(|_| ())
}

pub fn commit(checkout: &Path, message: &str, amend: bool) -> AppResult<()> {
    let message = message.trim();
    match (message.is_empty(), amend) {
        (true, false) => Err(AppError::Git("commit message is empty".into())),
        (true, true) => run(checkout, &["commit", "--amend", "--no-edit"]).map(|_| ()),
        (false, true) => run(checkout, &["commit", "--amend", "-m", message]).map(|_| ()),
        (false, false) => run(checkout, &["commit", "-m", message]).map(|_| ()),
    }
}

pub fn pull_ff(checkout: &Path) -> AppResult<String> {
    run_network(checkout, &["pull", "--ff-only"])
}

/// Pushes the checked-out branch, publishing it with `-u origin` when it has no upstream yet.
pub fn push(checkout: &Path, force_with_lease: bool) -> AppResult<String> {
    let branch = run_git(checkout, &["symbolic-ref", "--short", "-q", "HEAD"])
        .map_err(|_| AppError::Git("HEAD is detached; there is no branch to push".into()))?;
    let branch = branch.trim();
    let has_upstream = run_git(
        checkout,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )
    .is_ok();
    let mut args = vec!["push"];
    if force_with_lease {
        args.push("--force-with-lease");
    }
    if !has_upstream {
        args.extend(["-u", "origin", branch]);
    }
    run_network(checkout, &args)
}

pub fn stash_push(checkout: &Path, message: Option<&str>, include_untracked: bool) -> AppResult<()> {
    let mut args = vec!["stash", "push"];
    if include_untracked {
        args.push("-u");
    }
    if let Some(m) = message.map(str::trim).filter(|m| !m.is_empty()) {
        args.extend(["-m", m]);
    }
    let out = run(checkout, &args)?;
    if out.contains("No local changes to save") {
        return Err(AppError::Git("nothing to stash".into()));
    }
    Ok(())
}

pub fn stash_list(checkout: &Path) -> AppResult<Vec<StashEntry>> {
    let raw = run_git(checkout, &["stash", "list", "--format=%H%x1f%ct%x1f%gs"])?;
    Ok(raw
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let mut f = line.splitn(3, '\u{1f}');
            Some(StashEntry {
                index: index as u32,
                sha: f.next()?.to_string(),
                created_at: f.next()?.parse().ok()?,
                message: f.next()?.to_string(),
            })
        })
        .collect())
}

fn stash_ref(index: u32) -> String {
    format!("stash@{{{index}}}")
}

pub fn stash_apply(checkout: &Path, index: u32) -> AppResult<()> {
    run(checkout, &["stash", "apply", &stash_ref(index)]).map(|_| ())
}

pub fn stash_pop(checkout: &Path, index: u32) -> AppResult<()> {
    run(checkout, &["stash", "pop", &stash_ref(index)]).map(|_| ())
}

pub fn stash_drop(checkout: &Path, index: u32) -> AppResult<()> {
    run(checkout, &["stash", "drop", &stash_ref(index)]).map(|_| ())
}

pub fn stash_diff(checkout: &Path, index: u32) -> AppResult<String> {
    run_git(
        checkout,
        &["stash", "show", "-p", "--no-color", "--no-ext-diff", &stash_ref(index)],
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::process::Command;

    use tempfile::TempDir;

    use super::*;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn init() -> TempDir {
        let dir = TempDir::new().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        git(dir.path(), &["config", "user.name", "t"]);
        git(dir.path(), &["config", "user.email", "t@t"]);
        fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "init"]);
        dir
    }

    fn head_branch(dir: &Path) -> String {
        git(dir, &["rev-parse", "--abbrev-ref", "HEAD"]).trim().to_string()
    }

    fn err_text(r: AppResult<()>) -> String {
        r.unwrap_err().to_string()
    }

    #[test]
    fn classify_recognises_refusals() {
        assert_eq!(classify("error: The branch 'x' is not fully merged."), Some("not_fully_merged"));
        assert_eq!(classify("fatal: 'x' is already used by worktree at '/p'"), Some("checked_out_elsewhere"));
        assert_eq!(classify("Your local changes to the following files would be overwritten"), Some("dirty_tree"));
        assert_eq!(classify(" ! [rejected] main -> main (non-fast-forward)"), Some("push_rejected"));
        assert_eq!(classify("fatal: Not possible to fast-forward, aborting."), Some("not_fast_forward"));
        assert_eq!(classify("something else"), None);
    }

    #[test]
    fn branch_lifecycle_and_unmerged_delete() {
        let d = init();
        let p = d.path();
        create_branch(p, "feat", "HEAD", true).unwrap();
        assert_eq!(head_branch(p), "feat");
        fs::write(p.join("b.txt"), "b\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "b"]);
        checkout_branch(p, "main").unwrap();

        assert!(err_text(delete_branch(p, "feat", false)).starts_with("git error: [not_fully_merged]"));
        rename_branch(p, "feat", "feat2").unwrap();
        delete_branch(p, "feat2", true).unwrap();
        assert!(!git(p, &["branch", "--list", "feat2"]).contains("feat2"));
        assert!(create_branch(p, "bad name", "HEAD", false).is_err());
        assert!(create_branch(p, "-x", "HEAD", false).is_err());
    }

    #[test]
    fn dirty_checkout_is_refused() {
        let d = init();
        let p = d.path();
        create_branch(p, "other", "HEAD", true).unwrap();
        fs::write(p.join("a.txt"), "two\n").unwrap();
        git(p, &["commit", "-q", "-am", "change on other"]);
        checkout_branch(p, "main").unwrap();
        fs::write(p.join("a.txt"), "dirty\n").unwrap();
        assert!(err_text(checkout_branch(p, "other")).starts_with("git error: [dirty_tree]"));
    }

    #[test]
    fn branch_held_by_worktree_is_refused() {
        let d = init();
        let p = d.path();
        let wt = TempDir::new().unwrap();
        let wt_path = wt.path().join("wt");
        git(p, &["worktree", "add", "-q", "-b", "held", wt_path.to_str().unwrap()]);
        let msg = err_text(checkout_branch(p, "held"));
        assert!(msg.starts_with("git error: [checked_out_elsewhere]"), "{msg}");
    }

    #[test]
    fn detached_checkout_and_tags() {
        let d = init();
        let p = d.path();
        create_tag(p, "v1", "HEAD", None, false).unwrap();
        create_tag(p, "v2", "HEAD", Some("release"), false).unwrap();
        assert_eq!(git(p, &["cat-file", "-t", "v1"]).trim(), "commit");
        assert_eq!(git(p, &["cat-file", "-t", "v2"]).trim(), "tag");
        checkout_detached(p, "v2").unwrap();
        assert_eq!(head_branch(p), "HEAD");
        delete_tag(p, "v1").unwrap();
        assert!(git(p, &["tag", "--list", "v1"]).is_empty());
        assert!(create_tag(p, "bad..tag", "HEAD", None, false).is_err());
    }

    #[test]
    fn stage_unstage_discard_commit_amend() {
        let d = init();
        let p = d.path();
        fs::write(p.join("a.txt"), "two\n").unwrap();
        fs::write(p.join("new.txt"), "n\n").unwrap();

        stage(p, &["a.txt".into()]).unwrap();
        assert_eq!(git(p, &["diff", "--cached", "--name-only"]).trim(), "a.txt");
        unstage(p, &[]).unwrap();
        assert!(git(p, &["diff", "--cached", "--name-only"]).is_empty());

        discard(p, &["a.txt".into()], WipKind::Unstaged).unwrap();
        assert_eq!(fs::read_to_string(p.join("a.txt")).unwrap(), "one\n");
        discard(p, &["new.txt".into()], WipKind::Untracked).unwrap();
        assert!(!p.join("new.txt").exists());
        assert!(discard(p, &["a.txt".into()], WipKind::Staged).is_err());
        assert!(stage(p, &["../x".into()]).is_err());

        fs::write(p.join("a.txt"), "three\n").unwrap();
        stage(p, &[]).unwrap();
        assert!(commit(p, "  ", false).is_err());
        commit(p, "second", false).unwrap();
        commit(p, "second, reworded", true).unwrap();
        assert_eq!(git(p, &["log", "-1", "--format=%s"]).trim(), "second, reworded");
        assert_eq!(git(p, &["rev-list", "--count", "HEAD"]).trim(), "2");
    }

    #[test]
    fn unstage_works_on_unborn_head() {
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        fs::write(p.join("a.txt"), "x\n").unwrap();
        stage(p, &[]).unwrap();
        unstage(p, &["a.txt".into()]).unwrap();
        assert!(git(p, &["diff", "--cached", "--name-only"]).is_empty());
    }

    #[test]
    fn stash_roundtrip() {
        let d = init();
        let p = d.path();
        assert_eq!(err_text(stash_push(p, None, false)), "git error: nothing to stash");
        fs::write(p.join("a.txt"), "wip\n").unwrap();
        fs::write(p.join("u.txt"), "u\n").unwrap();
        stash_push(p, Some("my stash"), true).unwrap();
        assert!(!p.join("u.txt").exists());

        let list = stash_list(p).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].index, 0);
        assert!(list[0].message.contains("my stash"));
        assert!(stash_diff(p, 0).unwrap().contains("+wip"));

        stash_pop(p, 0).unwrap();
        assert_eq!(fs::read_to_string(p.join("a.txt")).unwrap(), "wip\n");
        assert!(stash_list(p).unwrap().is_empty());

        stash_push(p, None, false).unwrap();
        stash_drop(p, 0).unwrap();
        assert!(stash_list(p).unwrap().is_empty());
    }

    fn clone_with_remote() -> (TempDir, TempDir, TempDir) {
        let remote = TempDir::new().unwrap();
        git(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
        let a = TempDir::new().unwrap();
        git(a.path(), &["init", "-q", "-b", "main"]);
        fs::write(a.path().join("f"), "1\n").unwrap();
        git(a.path(), &["add", "."]);
        git(a.path(), &["commit", "-q", "-m", "init"]);
        git(a.path(), &["remote", "add", "origin", remote.path().to_str().unwrap()]);
        let b = TempDir::new().unwrap();
        (remote, a, b)
    }

    #[test]
    fn push_publishes_then_pull_ff_and_rejected_push() {
        let (remote, a, b) = clone_with_remote();
        push(a.path(), false).unwrap();
        assert!(git(a.path(), &["rev-parse", "--abbrev-ref", "@{u}"]).contains("origin/main"));

        git(b.path(), &["clone", "-q", remote.path().to_str().unwrap(), "."]);
        fs::write(a.path().join("f"), "2\n").unwrap();
        git(a.path(), &["commit", "-q", "-am", "two"]);
        push(a.path(), false).unwrap();
        pull_ff(b.path()).unwrap();
        assert_eq!(fs::read_to_string(b.path().join("f")).unwrap(), "2\n");

        fs::write(b.path().join("f"), "b-side\n").unwrap();
        git(b.path(), &["commit", "-q", "-am", "b"]);
        fs::write(a.path().join("f"), "3\n").unwrap();
        git(a.path(), &["commit", "-q", "-am", "three"]);
        push(a.path(), false).unwrap();

        let msg = push(b.path(), false).unwrap_err().to_string();
        assert!(msg.starts_with("git error: [push_rejected]"), "{msg}");
        let msg = pull_ff(b.path()).unwrap_err().to_string();
        assert!(msg.contains("[not_fast_forward]"), "{msg}");
        push(b.path(), true).unwrap();
    }

    #[test]
    fn tag_push_reaches_remote() {
        let (remote, a, _b) = clone_with_remote();
        create_tag(a.path(), "v9", "HEAD", None, true).unwrap();
        assert!(git(remote.path(), &["tag", "--list"]).contains("v9"));
    }
}
