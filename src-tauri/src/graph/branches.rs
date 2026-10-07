use std::path::Path;

use serde::Serialize;

use crate::error::AppResult;
use crate::git::run_git;

/// Local branches whose upstream no longer exists (`[gone]`), excluding the checked-out one.
pub fn list_gone(checkout: &Path) -> AppResult<Vec<String>> {
    let out = run_git(
        checkout,
        &[
            "for-each-ref",
            "--format=%(refname:short)%09%(upstream:track)%09%(HEAD)",
            "refs/heads",
        ],
    )?;
    Ok(out
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let name = parts.next()?;
            let track = parts.next().unwrap_or("");
            let head = parts.next().unwrap_or("");
            (track == "[gone]" && head.trim() != "*").then(|| name.to_string())
        })
        .collect())
}

/// `git fetch --prune` then report gone branches. Nothing is deleted.
pub fn gone_branches(checkout: &Path) -> AppResult<Vec<String>> {
    run_git(checkout, &["fetch", "--prune", "--quiet"])?;
    list_gone(checkout)
}

#[derive(Debug, Serialize)]
pub struct FailedDelete {
    pub name: String,
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct DeleteOutcome {
    pub deleted: Vec<String>,
    pub failed: Vec<FailedDelete>,
}

/// Force-deletes each requested branch that is *still* gone; anything else is ignored.
pub fn delete_gone_branches(checkout: &Path, names: &[String]) -> AppResult<DeleteOutcome> {
    let gone = list_gone(checkout)?;
    let mut outcome = DeleteOutcome { deleted: Vec::new(), failed: Vec::new() };
    for name in names.iter().filter(|n| gone.contains(n)) {
        match run_git(checkout, &["branch", "-D", "--", name]) {
            Ok(_) => outcome.deleted.push(name.clone()),
            Err(e) => outcome.failed.push(FailedDelete { name: name.clone(), error: e.to_string() }),
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn setup() -> (TempDir, TempDir) {
        let remote = TempDir::new().unwrap();
        git(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
        let local = TempDir::new().unwrap();
        let l = local.path();
        git(l, &["init", "-q", "-b", "main"]);
        git(l, &["config", "user.email", "t@example.com"]);
        git(l, &["config", "user.name", "T"]);
        git(l, &["config", "commit.gpgSign", "false"]);
        git(l, &["commit", "-q", "--allow-empty", "-m", "base"]);
        git(l, &["remote", "add", "origin", remote.path().to_str().unwrap()]);
        for b in ["main", "feat/gone", "kept"] {
            if b != "main" {
                git(l, &["branch", b]);
            }
            git(l, &["push", "-q", "-u", "origin", b]);
        }
        git(l, &["branch", "local-only"]);
        git(remote.path(), &["branch", "-D", "feat/gone"]);
        (remote, local)
    }

    #[test]
    fn finds_and_deletes_only_gone_branches() {
        let (_remote, local) = setup();
        let gone = gone_branches(local.path()).unwrap();
        assert_eq!(gone, vec!["feat/gone".to_string()]);

        let outcome = delete_gone_branches(
            local.path(),
            &["feat/gone".into(), "kept".into(), "local-only".into(), "-D".into()],
        )
        .unwrap();
        assert_eq!(outcome.deleted, vec!["feat/gone".to_string()]);
        assert!(outcome.failed.is_empty());
        let heads = git(local.path(), &["for-each-ref", "--format=%(refname:short)", "refs/heads"]);
        assert_eq!(heads.lines().collect::<Vec<_>>(), vec!["kept", "local-only", "main"]);
    }
}
