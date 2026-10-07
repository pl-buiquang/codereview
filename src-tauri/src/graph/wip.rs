use std::path::{Component, Path};
use std::process::Command;

use crate::error::{AppError, AppResult};
use crate::git::run_git;

use super::{ChangeStatus, ChangedFile, WipKind, WipStatus};

const DIFF_FLAGS: [&str; 4] = ["diff", "--no-color", "--no-ext-diff", "--no-textconv"];

pub fn worktree_status(checkout: &Path) -> AppResult<WipStatus> {
    // Unborn HEAD (fresh repo) is not an error for the WIP view; status below still validates the repo.
    let head_sha = run_git(checkout, &["rev-parse", "--verify", "-q", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let raw = run_git(
        checkout,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let (staged, unstaged, untracked) = parse_porcelain_z(&raw);
    Ok(WipStatus {
        head_sha,
        staged,
        unstaged,
        untracked,
    })
}

pub fn worktree_file_diff(checkout: &Path, path: &str, kind: WipKind) -> AppResult<String> {
    match kind {
        WipKind::Staged => {
            let mut args = DIFF_FLAGS.to_vec();
            args.extend(["--cached", "--", path]);
            run_git(checkout, &args)
        }
        WipKind::Unstaged => {
            let mut args = DIFF_FLAGS.to_vec();
            args.extend(["--", path]);
            run_git(checkout, &args)
        }
        WipKind::Untracked => {
            ensure_inside_checkout(path)?;
            let mut args = DIFF_FLAGS.to_vec();
            args.extend(["--no-index", "--", "/dev/null", path]);
            run_git_allow_diff_exit(checkout, &args)
        }
    }
}

/// `--no-index` reads arbitrary filesystem paths, so keep it confined to relative paths in the checkout.
fn ensure_inside_checkout(path: &str) -> AppResult<()> {
    let p = Path::new(path);
    let escapes = path.is_empty()
        || p.components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir));
    if escapes {
        return Err(AppError::Other(format!("invalid untracked path: {path}")));
    }
    Ok(())
}

/// Like `git::run_git`, but exit status 1 ("differences found") also counts as success.
fn run_git_allow_diff_exit(repo: &Path, args: &[&str]) -> AppResult<String> {
    let output = Command::new(crate::tools::git_bin())
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| AppError::Git(format!("failed to spawn git: {e}")))?;

    match output.status.code() {
        Some(0) | Some(1) => Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(AppError::Git(stderr.trim().to_string()))
        }
    }
}

fn status_from_code(c: u8) -> Option<ChangeStatus> {
    match c {
        b'A' => Some(ChangeStatus::Added),
        b'M' => Some(ChangeStatus::Modified),
        b'D' => Some(ChangeStatus::Deleted),
        b'R' => Some(ChangeStatus::Renamed),
        b'C' => Some(ChangeStatus::Copied),
        b'T' => Some(ChangeStatus::TypeChanged),
        _ => None,
    }
}

fn is_unmerged(x: u8, y: u8) -> bool {
    x == b'U' || y == b'U' || (x == b'A' && y == b'A') || (x == b'D' && y == b'D')
}

fn changed(path: &str, old_path: Option<&str>, status: ChangeStatus) -> ChangedFile {
    let old_path = matches!(status, ChangeStatus::Renamed | ChangeStatus::Copied)
        .then(|| old_path.map(str::to_string))
        .flatten();
    ChangedFile {
        path: path.to_string(),
        old_path,
        status,
        additions: None,
        deletions: None,
    }
}

type Grouped = (Vec<ChangedFile>, Vec<ChangedFile>, Vec<String>);

/// Parse `git status --porcelain=v1 -z`: `XY path\0`, with renames/copies followed by `orig\0`.
fn parse_porcelain_z(raw: &str) -> Grouped {
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    let mut untracked = Vec::new();
    let mut fields = raw.split('\0');

    while let Some(entry) = fields.next() {
        let bytes = entry.as_bytes();
        if bytes.len() < 4 {
            continue;
        }
        let (x, y) = (bytes[0], bytes[1]);
        let path = &entry[3..];

        let old_path = if matches!(x, b'R' | b'C') || matches!(y, b'R' | b'C') {
            fields.next()
        } else {
            None
        };

        match (x, y) {
            (b'?', b'?') => untracked.push(path.to_string()),
            (b'!', b'!') => {}
            _ if is_unmerged(x, y) => {
                unstaged.push(changed(path, None, ChangeStatus::Modified));
            }
            _ => {
                if let Some(s) = status_from_code(x) {
                    staged.push(changed(path, old_path, s));
                }
                if let Some(s) = status_from_code(y) {
                    unstaged.push(changed(path, old_path, s));
                }
            }
        }
    }
    (staged, unstaged, untracked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
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
    }

    fn fixture_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        git(p, &["config", "user.email", "test@example.com"]);
        git(p, &["config", "user.name", "Test"]);
        fs::write(p.join("file.txt"), "line1\nline2\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "initial"]);
        dir
    }

    #[test]
    fn clean_tree_is_empty() {
        let repo = fixture_repo();
        let st = worktree_status(repo.path()).unwrap();
        assert_eq!(st.head_sha.len(), 40);
        assert!(st.staged.is_empty());
        assert!(st.unstaged.is_empty());
        assert!(st.untracked.is_empty());
    }

    #[test]
    fn staged_only_change() {
        let repo = fixture_repo();
        let p = repo.path();
        fs::write(p.join("file.txt"), "line1\nline2\nstaged\n").unwrap();
        git(p, &["add", "file.txt"]);

        let st = worktree_status(p).unwrap();
        assert_eq!(st.staged.len(), 1);
        assert_eq!(st.staged[0].path, "file.txt");
        assert_eq!(st.staged[0].status, ChangeStatus::Modified);
        assert!(st.unstaged.is_empty());
        assert!(st.untracked.is_empty());

        let diff = worktree_file_diff(p, "file.txt", WipKind::Staged).unwrap();
        assert!(diff.contains("+staged"), "diff was: {diff}");
        let none = worktree_file_diff(p, "file.txt", WipKind::Unstaged).unwrap();
        assert!(none.trim().is_empty(), "unexpected unstaged diff: {none}");
    }

    #[test]
    fn unstaged_only_change() {
        let repo = fixture_repo();
        let p = repo.path();
        fs::write(p.join("file.txt"), "line1\nchanged\n").unwrap();

        let st = worktree_status(p).unwrap();
        assert!(st.staged.is_empty());
        assert_eq!(st.unstaged.len(), 1);
        assert_eq!(st.unstaged[0].path, "file.txt");
        assert_eq!(st.unstaged[0].status, ChangeStatus::Modified);
        assert!(st.unstaged[0].old_path.is_none());

        let diff = worktree_file_diff(p, "file.txt", WipKind::Unstaged).unwrap();
        assert!(diff.contains("+changed"), "diff was: {diff}");
        assert!(diff.contains("-line2"), "diff was: {diff}");
    }

    #[test]
    fn untracked_file_and_its_diff() {
        let repo = fixture_repo();
        let p = repo.path();
        fs::create_dir(p.join("sub")).unwrap();
        fs::write(p.join("sub/new file.txt"), "hello\nworld\n").unwrap();

        let st = worktree_status(p).unwrap();
        assert!(st.staged.is_empty());
        assert!(st.unstaged.is_empty());
        assert_eq!(st.untracked, vec!["sub/new file.txt".to_string()]);

        let diff = worktree_file_diff(p, "sub/new file.txt", WipKind::Untracked).unwrap();
        assert!(!diff.is_empty());
        assert!(diff.contains("+hello"), "diff was: {diff}");
        assert!(diff.contains("+world"), "diff was: {diff}");
    }

    #[test]
    fn untracked_diff_rejects_escaping_paths() {
        let repo = fixture_repo();
        for bad in ["../outside.txt", "/etc/hosts", ""] {
            let err = worktree_file_diff(repo.path(), bad, WipKind::Untracked).unwrap_err();
            assert!(matches!(err, AppError::Other(_)), "{bad}: {err:?}");
        }
    }

    #[test]
    fn renamed_staged_reports_old_path() {
        let repo = fixture_repo();
        let p = repo.path();
        git(p, &["mv", "file.txt", "renamed.txt"]);

        let st = worktree_status(p).unwrap();
        assert_eq!(st.staged.len(), 1);
        let f = &st.staged[0];
        assert_eq!(f.status, ChangeStatus::Renamed);
        assert_eq!(f.path, "renamed.txt");
        assert_eq!(f.old_path.as_deref(), Some("file.txt"));
        assert!(st.unstaged.is_empty());
        assert!(st.untracked.is_empty());
    }

    #[test]
    fn parses_mixed_porcelain_entries() {
        let raw = "MM both.txt\0R  new.txt\0old.txt\0?? u.txt\0 D gone.txt\0UU conflict.txt\0";
        let (staged, unstaged, untracked) = parse_porcelain_z(raw);
        assert_eq!(
            staged.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            vec!["both.txt", "new.txt"]
        );
        assert_eq!(staged[1].old_path.as_deref(), Some("old.txt"));
        assert_eq!(
            unstaged
                .iter()
                .map(|f| (f.path.as_str(), f.status))
                .collect::<Vec<_>>(),
            vec![
                ("both.txt", ChangeStatus::Modified),
                ("gone.txt", ChangeStatus::Deleted),
                ("conflict.txt", ChangeStatus::Modified),
            ]
        );
        assert_eq!(untracked, vec!["u.txt".to_string()]);
    }
}
