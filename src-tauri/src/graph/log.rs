use std::path::Path;

use crate::error::{AppError, AppResult};
use crate::git::run_git;

use super::{GraphCommit, RefInfo, RefKind, Scope};

const LOG_FORMAT: &str = "--format=%H%x1f%P%x1f%an%x1f%at%x1f%ct%x1f%s%x1f%b%x1e";
const REF_FORMAT: &str = "--format=%(refname)%09%(objectname)%09%(*objectname)%09%(HEAD)%09%(symref)";

pub fn graph_log(
    checkout: &Path,
    scope: Scope,
    skip: u32,
    limit: u32,
) -> AppResult<Vec<GraphCommit>> {
    let has_head = run_git(checkout, &["rev-parse", "--verify", "-q", "HEAD"]).is_ok();
    let mut revs: Vec<&str> = match scope {
        Scope::All => vec!["--branches", "--remotes", "--tags"],
        Scope::Local => vec!["--branches"],
        Scope::Current => vec![],
    };
    if has_head {
        revs.push("HEAD");
    } else if scope == Scope::Current {
        return Ok(Vec::new());
    }

    let skip_arg = format!("--skip={skip}");
    let limit_arg = format!("--max-count={limit}");
    let mut args = vec![
        "log",
        "--topo-order",
        "--no-color",
        LOG_FORMAT,
        skip_arg.as_str(),
        limit_arg.as_str(),
    ];
    args.extend(revs);
    args.push("--");

    let out = run_git(checkout, &args)?;
    out.split('\x1e')
        .map(|rec| rec.trim_start_matches(['\n', '\r']))
        .filter(|rec| !rec.is_empty())
        .map(parse_commit)
        .collect()
}

fn parse_commit(rec: &str) -> AppResult<GraphCommit> {
    let fields: Vec<&str> = rec.splitn(7, '\x1f').collect();
    if fields.len() != 7 {
        return Err(AppError::Git(format!("malformed git log record: {rec:?}")));
    }
    let parse_time = |s: &str| {
        s.trim()
            .parse::<i64>()
            .map_err(|_| AppError::Git(format!("bad timestamp in git log: {s:?}")))
    };
    Ok(GraphCommit {
        sha: fields[0].to_string(),
        parents: fields[1].split_whitespace().map(str::to_string).collect(),
        author_name: fields[2].to_string(),
        author_time: parse_time(fields[3])?,
        committer_time: parse_time(fields[4])?,
        subject: fields[5].to_string(),
        body_preview: fields[6]
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("")
            .to_string(),
    })
}

pub fn list_refs(checkout: &Path) -> AppResult<Vec<RefInfo>> {
    let out = run_git(
        checkout,
        &[
            "for-each-ref",
            REF_FORMAT,
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ],
    )?;
    Ok(out.lines().filter_map(parse_ref).collect())
}

fn parse_ref(line: &str) -> Option<RefInfo> {
    let mut parts = line.split('\t');
    let refname = parts.next()?;
    let object = parts.next()?;
    let peeled = parts.next().unwrap_or("");
    let head = parts.next().unwrap_or("");
    let symref = parts.next().unwrap_or("");
    if !symref.is_empty() {
        return None;
    }
    let (kind, name) = if let Some(n) = refname.strip_prefix("refs/heads/") {
        (RefKind::Local, n)
    } else if let Some(n) = refname.strip_prefix("refs/remotes/") {
        (RefKind::Remote, n)
    } else if let Some(n) = refname.strip_prefix("refs/tags/") {
        (RefKind::Tag, n)
    } else {
        return None;
    };
    let sha = if peeled.is_empty() { object } else { peeled };
    Some(RefInfo {
        name: name.to_string(),
        kind,
        sha: sha.to_string(),
        is_head: head.trim() == "*",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    struct Fixture {
        dir: TempDir,
        clock: Cell<i64>,
    }

    impl Fixture {
        fn new() -> Self {
            let f = Fixture {
                dir: TempDir::new().unwrap(),
                clock: Cell::new(1_700_000_000),
            };
            f.git(&["init", "-q", "-b", "main"]);
            f.git(&["config", "user.email", "test@example.com"]);
            f.git(&["config", "user.name", "Test"]);
            f.git(&["config", "tag.gpgSign", "false"]);
            f.git(&["config", "commit.gpgSign", "false"]);
            f
        }

        fn path(&self) -> &Path {
            self.dir.path()
        }

        fn git(&self, args: &[&str]) -> String {
            let t = self.clock.get() + 60;
            self.clock.set(t);
            let date = format!("@{t} +0000");
            let out = Command::new("git")
                .arg("-C")
                .arg(self.path())
                .args(args)
                .env("GIT_AUTHOR_DATE", &date)
                .env("GIT_COMMITTER_DATE", &date)
                .output()
                .expect("spawn git");
            assert!(
                out.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }

        fn commit(&self, file: &str, msg: &str) -> String {
            fs::write(self.path().join(file), msg).unwrap();
            self.git(&["add", "."]);
            self.git(&["commit", "-q", "-m", msg]);
            self.git(&["rev-parse", "HEAD"])
        }
    }

    fn shas(commits: &[GraphCommit]) -> Vec<String> {
        commits.iter().map(|c| c.sha.clone()).collect()
    }

    #[test]
    fn linear_history_newest_first_with_fields() {
        let f = Fixture::new();
        let c1 = f.commit("a.txt", "first");
        let c2 = f.commit("a.txt", "second\n\n\nbody line one\nbody line two");
        let c3 = f.commit("a.txt", "third");

        let log = graph_log(f.path(), Scope::Current, 0, 100).unwrap();
        assert_eq!(shas(&log), vec![c3.clone(), c2.clone(), c1.clone()]);

        assert_eq!(log[0].subject, "third");
        assert_eq!(log[0].body_preview, "");
        assert_eq!(log[0].parents, vec![c2.clone()]);
        assert_eq!(log[1].subject, "second");
        assert_eq!(log[1].body_preview, "body line one");
        assert_eq!(log[2].parents, Vec::<String>::new());
        assert_eq!(log[0].author_name, "Test");
        assert!(log[0].author_time > log[2].author_time);
        assert!(log[0].committer_time >= log[0].author_time);
    }

    #[test]
    fn merge_commit_keeps_parent_order() {
        let f = Fixture::new();
        f.commit("a.txt", "base");
        f.git(&["checkout", "-q", "-b", "feature"]);
        let feat = f.commit("b.txt", "feature work");
        f.git(&["checkout", "-q", "main"]);
        let main_tip = f.commit("c.txt", "main work");
        f.git(&["merge", "-q", "--no-ff", "-m", "merge feature", "feature"]);
        let merge = f.git(&["rev-parse", "HEAD"]);

        let log = graph_log(f.path(), Scope::Current, 0, 100).unwrap();
        assert_eq!(log.len(), 4);
        assert_eq!(log[0].sha, merge);
        assert_eq!(log[0].parents, vec![main_tip, feat]);
        assert_eq!(log[0].subject, "merge feature");
    }

    #[test]
    fn tags_lightweight_and_annotated_are_peeled() {
        let f = Fixture::new();
        let c1 = f.commit("a.txt", "one");
        let c2 = f.commit("a.txt", "two");
        f.git(&["tag", "light", &c1]);
        f.git(&["tag", "-a", "v1.0", "-m", "release", &c2]);
        let tag_obj = f.git(&["rev-parse", "v1.0"]);
        assert_ne!(tag_obj, c2);

        let refs = list_refs(f.path()).unwrap();
        let light = refs.iter().find(|r| r.name == "light").unwrap();
        assert_eq!(light.kind, RefKind::Tag);
        assert_eq!(light.sha, c1);
        let annotated = refs.iter().find(|r| r.name == "v1.0").unwrap();
        assert_eq!(annotated.kind, RefKind::Tag);
        assert_eq!(annotated.sha, c2);
        assert!(!annotated.is_head);
    }

    #[test]
    fn refs_kinds_head_and_symbolic_skipped() {
        let f = Fixture::new();
        let c1 = f.commit("a.txt", "one");
        f.git(&["branch", "other"]);
        f.git(&["update-ref", "refs/remotes/origin/main", &c1]);
        f.git(&[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ]);

        let refs = list_refs(f.path()).unwrap();
        let names: Vec<&str> = refs.iter().map(|r| r.name.as_str()).collect();
        assert!(!names.contains(&"origin/HEAD"), "refs: {names:?}");

        let main = refs.iter().find(|r| r.name == "main").unwrap();
        assert_eq!(main.kind, RefKind::Local);
        assert!(main.is_head);
        assert_eq!(main.sha, c1);
        let other = refs.iter().find(|r| r.name == "other").unwrap();
        assert!(!other.is_head);
        let remote = refs.iter().find(|r| r.name == "origin/main").unwrap();
        assert_eq!(remote.kind, RefKind::Remote);
        assert_eq!(remote.sha, c1);
        assert_eq!(refs.len(), 3);
    }

    #[test]
    fn paging_returns_disjoint_consecutive_slices() {
        let f = Fixture::new();
        for i in 0..7 {
            f.commit("a.txt", &format!("c{i}"));
        }
        let all = graph_log(f.path(), Scope::Current, 0, 100).unwrap();
        assert_eq!(all.len(), 7);

        let p1 = graph_log(f.path(), Scope::Current, 0, 3).unwrap();
        let p2 = graph_log(f.path(), Scope::Current, 3, 3).unwrap();
        let p3 = graph_log(f.path(), Scope::Current, 6, 3).unwrap();
        let p4 = graph_log(f.path(), Scope::Current, 9, 3).unwrap();
        assert_eq!(p1.len(), 3);
        assert_eq!(p2.len(), 3);
        assert_eq!(p3.len(), 1);
        assert!(p4.is_empty());

        let stitched: Vec<String> = [p1, p2, p3].iter().flat_map(|p| shas(p)).collect();
        assert_eq!(stitched, shas(&all));
    }

    #[test]
    fn scopes_select_different_commit_sets() {
        let f = Fixture::new();
        let base = f.commit("a.txt", "base");

        f.git(&["checkout", "-q", "-b", "side"]);
        let side = f.commit("s.txt", "side work");
        f.git(&["checkout", "-q", "main"]);

        f.git(&["checkout", "-q", "-b", "tmp"]);
        let remote_only = f.commit("r.txt", "remote work");
        f.git(&["update-ref", "refs/remotes/origin/feat", &remote_only]);
        f.git(&["checkout", "-q", "main"]);
        f.git(&["branch", "-q", "-D", "tmp"]);

        f.git(&["checkout", "-q", "--orphan", "tagged"]);
        let tag_only = f.commit("t.txt", "tag only");
        f.git(&["tag", "-a", "orphan-tag", "-m", "t", &tag_only]);
        f.git(&["checkout", "-q", "-f", "main"]);
        f.git(&["branch", "-q", "-D", "tagged"]);

        let current = shas(&graph_log(f.path(), Scope::Current, 0, 100).unwrap());
        assert_eq!(current, vec![base.clone()]);

        let local = shas(&graph_log(f.path(), Scope::Local, 0, 100).unwrap());
        assert!(local.contains(&base) && local.contains(&side));
        assert!(!local.contains(&remote_only));
        assert!(!local.contains(&tag_only));
        assert_eq!(local.len(), 2);

        let all = shas(&graph_log(f.path(), Scope::All, 0, 100).unwrap());
        assert!(all.contains(&remote_only));
        assert!(all.contains(&tag_only));
        assert_eq!(all.len(), 4);
    }

    #[test]
    fn detached_head_included_in_local_scope() {
        let f = Fixture::new();
        f.commit("a.txt", "base");
        f.git(&["checkout", "-q", "--detach"]);
        let detached = f.commit("a.txt", "detached work");

        let local = shas(&graph_log(f.path(), Scope::Local, 0, 100).unwrap());
        assert_eq!(local[0], detached);
    }

    #[test]
    fn unborn_head_yields_empty_log() {
        let f = Fixture::new();
        assert!(graph_log(f.path(), Scope::Current, 0, 10).unwrap().is_empty());
        assert!(graph_log(f.path(), Scope::All, 0, 10).unwrap().is_empty());
        assert!(list_refs(f.path()).unwrap().is_empty());
    }
}
