use std::collections::HashMap;
use std::path::Path;

use crate::error::{AppError, AppResult};
use crate::git::{rev_parse, run_git};

use super::{ChangeStatus, ChangedFile, CommitDetail};

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

const HEADER_FORMAT: &str =
    "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%cn%x1f%ce%x1f%ct%x1f%s%x1f%b";

pub fn commit_detail(checkout: &Path, sha: &str) -> AppResult<CommitDetail> {
    let sha = rev_parse(checkout, sha)?;
    let raw = run_git(
        checkout,
        &["show", "-s", "--no-color", HEADER_FORMAT, &sha, "--"],
    )?;
    let mut detail = parse_header(&raw)?;
    let base = diff_base(&detail.parents);

    let name_status = run_git(
        checkout,
        &[
            "diff-tree",
            "-r",
            "-M",
            "-z",
            "--name-status",
            base,
            &sha,
            "--",
        ],
    )?;
    let numstat = run_git(
        checkout,
        &["diff-tree", "-r", "-M", "-z", "--numstat", base, &sha, "--"],
    )?;
    let counts = parse_numstat(&numstat);
    detail.files = parse_name_status(&name_status)?
        .into_iter()
        .map(|mut f| {
            if let Some(&(a, d)) = counts.get(&f.path) {
                f.additions = a;
                f.deletions = d;
            }
            f
        })
        .collect();
    Ok(detail)
}

pub fn commit_file_diff(
    checkout: &Path,
    sha: &str,
    path: &str,
    old_path: Option<&str>,
) -> AppResult<String> {
    let sha = rev_parse(checkout, sha)?;
    let parents = run_git(checkout, &["show", "-s", "--format=%P", &sha, "--"])?;
    let parents: Vec<String> = parents.split_whitespace().map(str::to_string).collect();
    let base = diff_base(&parents);

    let mut args = vec![
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        "-M",
        base,
        &sha,
        "--",
    ];
    if let Some(old) = old_path.filter(|o| *o != path) {
        args.push(old);
    }
    args.push(path);
    run_git(checkout, &args)
}

/// Merges diff against their first parent; root commits against the empty tree.
fn diff_base(parents: &[String]) -> &str {
    parents.first().map(String::as_str).unwrap_or(EMPTY_TREE)
}

fn parse_header(raw: &str) -> AppResult<CommitDetail> {
    let fields: Vec<&str> = raw.splitn(10, '\x1f').collect();
    if fields.len() != 10 {
        return Err(AppError::Git("unexpected git show output".into()));
    }
    let time = |s: &str| {
        s.trim()
            .parse::<i64>()
            .map_err(|_| AppError::Git(format!("invalid timestamp: {s}")))
    };
    Ok(CommitDetail {
        sha: fields[0].trim().to_string(),
        parents: fields[1].split_whitespace().map(str::to_string).collect(),
        author_name: fields[2].to_string(),
        author_email: fields[3].to_string(),
        author_time: time(fields[4])?,
        committer_name: fields[5].to_string(),
        committer_email: fields[6].to_string(),
        committer_time: time(fields[7])?,
        subject: fields[8].to_string(),
        body: fields[9].trim_end().to_string(),
        files: Vec::new(),
    })
}

fn parse_status(code: &str) -> AppResult<ChangeStatus> {
    Ok(match code.chars().next() {
        Some('A') => ChangeStatus::Added,
        Some('M') => ChangeStatus::Modified,
        Some('D') => ChangeStatus::Deleted,
        Some('R') => ChangeStatus::Renamed,
        Some('C') => ChangeStatus::Copied,
        Some('T') => ChangeStatus::TypeChanged,
        _ => return Err(AppError::Git(format!("unknown diff status: {code}"))),
    })
}

/// Parses `diff-tree -z --name-status`: `STATUS\0path\0`, or `R100\0old\0new\0` for renames/copies.
fn parse_name_status(raw: &str) -> AppResult<Vec<ChangedFile>> {
    let mut tokens = raw.split('\0').filter(|t| !t.is_empty());
    let mut files = Vec::new();
    while let Some(code) = tokens.next() {
        let status = parse_status(code)?;
        let first = tokens
            .next()
            .ok_or_else(|| AppError::Git("truncated diff-tree output".into()))?;
        let (path, old_path) = if matches!(status, ChangeStatus::Renamed | ChangeStatus::Copied) {
            let new = tokens
                .next()
                .ok_or_else(|| AppError::Git("truncated diff-tree output".into()))?;
            (new.to_string(), Some(first.to_string()))
        } else {
            (first.to_string(), None)
        };
        files.push(ChangedFile {
            path,
            old_path,
            status,
            additions: None,
            deletions: None,
        });
    }
    Ok(files)
}

type Counts = (Option<u32>, Option<u32>);

/// Parses `diff-tree -z --numstat`, keyed by new path. Renames emit
/// `add\tdel\t\0old\0new\0`; binary files report `-` for both counts.
fn parse_numstat(raw: &str) -> HashMap<String, Counts> {
    let mut out = HashMap::new();
    let mut tokens = raw.split('\0');
    while let Some(tok) = tokens.next() {
        if tok.is_empty() {
            continue;
        }
        let mut parts = tok.splitn(3, '\t');
        let (Some(add), Some(del), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let path = if path.is_empty() {
            let _old = tokens.next();
            match tokens.next() {
                Some(new) => new,
                None => break,
            }
        } else {
            path
        };
        out.insert(path.to_string(), (add.parse().ok(), del.parse().ok()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    fn git(dir: &Path, args: &[&str]) -> String {
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
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn init_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        git(p, &["config", "user.email", "author@example.com"]);
        git(p, &["config", "user.name", "Author"]);
        git(p, &["config", "commit.gpgsign", "false"]);
        dir
    }

    fn commit_all(p: &Path, msg: &str) -> String {
        git(p, &["add", "-A"]);
        git(p, &["commit", "-q", "-m", msg]);
        git(p, &["rev-parse", "HEAD"])
    }

    fn file<'a>(d: &'a CommitDetail, path: &str) -> &'a ChangedFile {
        d.files
            .iter()
            .find(|f| f.path == path)
            .unwrap_or_else(|| panic!("{path} not in {:?}", d.files))
    }

    #[test]
    fn root_commit_diffs_against_empty_tree() {
        let dir = init_repo();
        let p = dir.path();
        fs::write(p.join("a.txt"), "one\ntwo\n").unwrap();
        fs::write(p.join("bin.dat"), [0u8, 1, 2, 0, 255]).unwrap();
        let sha = commit_all(p, "initial\n\nbody line 1\nbody line 2");

        let d = commit_detail(p, &sha).unwrap();
        assert_eq!(d.sha, sha);
        assert!(d.parents.is_empty());
        assert_eq!(d.subject, "initial");
        assert_eq!(d.body, "body line 1\nbody line 2");
        assert_eq!(d.author_name, "Author");
        assert_eq!(d.author_email, "author@example.com");
        assert_eq!(d.committer_name, "Author");
        assert!(d.author_time > 0 && d.committer_time > 0);
        assert_eq!(d.files.len(), 2);
        let a = file(&d, "a.txt");
        assert_eq!(a.status, ChangeStatus::Added);
        assert_eq!((a.additions, a.deletions), (Some(2), Some(0)));
        let bin = file(&d, "bin.dat");
        assert_eq!((bin.additions, bin.deletions), (None, None));

        let diff = commit_file_diff(p, &sha, "a.txt", None).unwrap();
        assert!(diff.contains("new file mode"));
        assert!(diff.contains("+one"));
    }

    #[test]
    fn normal_modify_commit() {
        let dir = init_repo();
        let p = dir.path();
        fs::write(p.join("a.txt"), "one\ntwo\n").unwrap();
        fs::write(p.join("gone.txt"), "x\n").unwrap();
        let first = commit_all(p, "initial");
        fs::write(p.join("a.txt"), "one\nTWO\nthree\n").unwrap();
        fs::remove_file(p.join("gone.txt")).unwrap();
        let sha = commit_all(p, "modify");

        let d = commit_detail(p, &sha).unwrap();
        assert_eq!(d.parents, vec![first]);
        assert_eq!(d.body, "");
        let a = file(&d, "a.txt");
        assert_eq!(a.status, ChangeStatus::Modified);
        assert_eq!(a.old_path, None);
        assert_eq!((a.additions, a.deletions), (Some(2), Some(1)));
        assert_eq!(file(&d, "gone.txt").status, ChangeStatus::Deleted);

        let short = &sha[..10];
        let diff = commit_file_diff(p, short, "a.txt", None).unwrap();
        assert!(diff.contains("-two"));
        assert!(diff.contains("+TWO"));
        assert!(!diff.contains("gone.txt"));
    }

    #[test]
    fn merge_commit_diffs_against_first_parent_only() {
        let dir = init_repo();
        let p = dir.path();
        fs::write(p.join("base.txt"), "base\n").unwrap();
        commit_all(p, "initial");
        git(p, &["checkout", "-q", "-b", "feature"]);
        fs::write(p.join("feature.txt"), "feature\n").unwrap();
        commit_all(p, "feature work");
        git(p, &["checkout", "-q", "main"]);
        fs::write(p.join("main.txt"), "main\n").unwrap();
        let main_tip = commit_all(p, "main work");
        git(
            p,
            &["merge", "-q", "--no-ff", "-m", "merge feature", "feature"],
        );
        let merge = git(p, &["rev-parse", "HEAD"]);

        let d = commit_detail(p, &merge).unwrap();
        assert_eq!(d.parents.len(), 2);
        assert_eq!(d.parents[0], main_tip);
        assert_eq!(d.files.len(), 1, "{:?}", d.files);
        assert_eq!(file(&d, "feature.txt").status, ChangeStatus::Added);

        let diff = commit_file_diff(p, &merge, "feature.txt", None).unwrap();
        assert!(diff.contains("+feature"));
        assert!(commit_file_diff(p, &merge, "main.txt", None)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn rename_reports_old_path_and_diffs_as_rename() {
        let dir = init_repo();
        let p = dir.path();
        let content: String = (0..20).map(|i| format!("line {i}\n")).collect();
        fs::write(p.join("old name.txt"), &content).unwrap();
        commit_all(p, "initial");
        fs::remove_file(p.join("old name.txt")).unwrap();
        fs::create_dir(p.join("dir")).unwrap();
        fs::write(p.join("dir/new name.txt"), content + "extra\n").unwrap();
        let sha = commit_all(p, "rename");

        let d = commit_detail(p, &sha).unwrap();
        assert_eq!(d.files.len(), 1, "{:?}", d.files);
        let f = &d.files[0];
        assert_eq!(f.status, ChangeStatus::Renamed);
        assert_eq!(f.path, "dir/new name.txt");
        assert_eq!(f.old_path.as_deref(), Some("old name.txt"));
        assert_eq!((f.additions, f.deletions), (Some(1), Some(0)));

        let diff = commit_file_diff(p, &sha, &f.path, f.old_path.as_deref()).unwrap();
        assert!(diff.contains("rename from old name.txt"), "{diff}");
        assert!(diff.contains("rename to dir/new name.txt"), "{diff}");
        assert!(diff.contains("+extra"));
    }

    #[test]
    fn invalid_sha_is_rejected() {
        let dir = init_repo();
        let p = dir.path();
        fs::write(p.join("a.txt"), "a\n").unwrap();
        commit_all(p, "initial");

        for bad in ["", "--output=/tmp/x", "HEAD\nmain", "deadbeefdeadbeef"] {
            assert!(commit_detail(p, bad).is_err(), "{bad:?} accepted");
            assert!(
                commit_file_diff(p, bad, "a.txt", None).is_err(),
                "{bad:?} accepted"
            );
        }
    }

    #[test]
    fn parse_numstat_handles_renames_and_binary() {
        let raw = concat!("1\t2\ta.txt\0", "-\t-\tbin\0", "3\t0\t\0old\0new\0");
        let m = parse_numstat(raw);
        assert_eq!(m["a.txt"], (Some(1), Some(2)));
        assert_eq!(m["bin"], (None, None));
        assert_eq!(m["new"], (Some(3), Some(0)));
        assert!(!m.contains_key("old"));
    }

    #[test]
    fn parse_name_status_handles_copy_score() {
        let files = parse_name_status("C075\0src.rs\0dst.rs\0T\0link\0").unwrap();
        assert_eq!(files[0].status, ChangeStatus::Copied);
        assert_eq!(files[0].path, "dst.rs");
        assert_eq!(files[0].old_path.as_deref(), Some("src.rs"));
        assert_eq!(files[1].status, ChangeStatus::TypeChanged);
        assert_eq!(files[1].old_path, None);
    }
}
