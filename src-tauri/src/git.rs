use std::path::Path;
use std::process::Command;

use serde::Serialize;

use crate::error::{AppError, AppResult};

/// Run a git command inside `repo` and return stdout on success.
pub fn run_git(repo: &Path, args: &[&str]) -> AppResult<String> {
    run_git_env(repo, args, &[])
}

/// [`run_git`] with extra environment variables for the child process.
pub fn run_git_env(repo: &Path, args: &[&str], env: &[(&str, &str)]) -> AppResult<String> {
    // Reads must not write: `git status` would otherwise refresh the index,
    // which the repo watcher sees as a change and reacts to with another status.
    let started = std::time::Instant::now();
    let output = Command::new(crate::tools::git_bin())
        .env("GIT_OPTIONAL_LOCKS", "0")
        .envs(env.iter().copied())
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| {
            log::warn!(target: "git", "failed to spawn git in {}: {e}", repo.display());
            AppError::Git(format!("failed to spawn git: {e}"))
        })?;

    let ms = started.elapsed().as_millis();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::debug!(
            target: "git",
            "`git {}` in {} failed ({}) after {ms}ms: {}",
            args.join(" "),
            repo.display(),
            output.status,
            stderr.trim()
        );
        return Err(AppError::Git(stderr.trim().to_string()));
    }
    log::debug!(
        target: "git",
        "`git {}` in {} ok in {ms}ms ({} bytes)",
        args.join(" "),
        repo.display(),
        output.stdout.len()
    );
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// [`run_git`] returning raw stdout, for binary blobs (`git cat-file blob`).
pub fn run_git_bytes(repo: &Path, args: &[&str]) -> AppResult<Vec<u8>> {
    let output = Command::new(crate::tools::git_bin())
        .env("GIT_OPTIONAL_LOCKS", "0")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| AppError::Git(format!("failed to spawn git: {e}")))?;
    if !output.status.success() {
        return Err(AppError::Git(String::from_utf8_lossy(&output.stderr).trim().to_string()));
    }
    Ok(output.stdout)
}

/// Raw bytes of `path` at `rev` (`git cat-file blob rev:path`).
pub fn show_blob_bytes(repo: &Path, rev: &str, path: &str) -> AppResult<Vec<u8>> {
    validate_commit_ref(rev)?;
    run_git_bytes(repo, &["cat-file", "blob", &format!("{rev}:{path}")])
}

/// Whether a background fetch ([`fetch_commits_in_background`]) is running in `repo`.
pub fn background_fetch_running(repo: &Path) -> bool {
    BG_FETCHES.lock().unwrap().as_ref().is_some_and(|set| set.contains(repo))
}

pub fn is_git_repo(path: &Path) -> bool {
    run_git(path, &["rev-parse", "--is-inside-work-tree"])
        .map(|s| s.trim() == "true")
        .unwrap_or(false)
}

#[derive(Debug, Default)]
pub struct RemoteInfo {
    pub owner: Option<String>,
    pub name: Option<String>,
}

/// Best-effort parse of `origin`'s owner/repo (GitHub ssh or https form).
pub fn remote_info(path: &Path) -> RemoteInfo {
    match run_git(path, &["remote", "get-url", "origin"]) {
        Ok(url) => {
            let info = parse_owner_repo(&url);
            log::debug!(
                target: "git",
                "remote_info {}: origin {:?} -> owner {:?}, name {:?}",
                path.display(),
                url.trim(),
                info.owner,
                info.name
            );
            info
        }
        Err(e) => {
            log::debug!(target: "git", "remote_info {}: no origin ({e})", path.display());
            RemoteInfo::default()
        }
    }
}

pub fn parse_owner_repo(url: &str) -> RemoteInfo {
    let u = url.trim().trim_end_matches(".git");
    // For `git@github.com:owner/repo` or `https://github.com/owner/repo`,
    // take everything after the host, then the last two path segments.
    let tail = match u.find("github.com") {
        Some(idx) => u[idx + "github.com".len()..].trim_start_matches([':', '/']),
        None => u,
    };
    let parts: Vec<&str> = tail.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() >= 2 {
        RemoteInfo {
            owner: Some(parts[parts.len() - 2].to_string()),
            name: Some(parts[parts.len() - 1].to_string()),
        }
    } else {
        RemoteInfo::default()
    }
}

/// Resolve the repository's default branch (origin/HEAD, falling back to the
/// current branch).
pub fn default_branch(path: &Path) -> Option<String> {
    run_git(
        path,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    )
    .ok()
    .map(|s| s.trim().trim_start_matches("origin/").to_string())
    .filter(|s| !s.is_empty())
    .or_else(|| {
        run_git(path, &["rev-parse", "--abbrev-ref", "HEAD"])
            .ok()
            .map(|s| s.trim().to_string())
    })
}

#[derive(Debug, Serialize)]
pub struct Branch {
    pub name: String,
    pub is_remote: bool,
    pub sha: String,
}

/// List local and remote-tracking branches (remote HEAD pointers excluded).
pub fn list_branches(path: &Path) -> AppResult<Vec<Branch>> {
    let out = run_git(
        path,
        &[
            "for-each-ref",
            "--format=%(refname:short)%09%(objectname)%09%(refname)",
            "refs/heads",
            "refs/remotes",
        ],
    )?;
    let mut branches = Vec::new();
    for line in out.lines() {
        let mut cols = line.split('\t');
        let (Some(name), Some(sha), Some(full)) = (cols.next(), cols.next(), cols.next()) else {
            continue;
        };
        // Skip symbolic refs like `origin/HEAD`.
        if name.ends_with("/HEAD") {
            continue;
        }
        branches.push(Branch {
            name: name.to_string(),
            is_remote: full.starts_with("refs/remotes"),
            sha: sha.to_string(),
        });
    }
    Ok(branches)
}

pub fn validate_commit_ref(rev: &str) -> AppResult<()> {
    if rev.is_empty() || rev.starts_with('-') || rev.chars().any(char::is_control) {
        return Err(AppError::Git("invalid commit reference".into()));
    }
    Ok(())
}

/// Whether `sha` is already a local commit object.
pub fn has_commit(repo: &Path, sha: &str) -> bool {
    validate_commit_ref(sha).is_ok() && run_git(repo, &["cat-file", "-e", &format!("{sha}^{{commit}}")]).is_ok()
}

/// Fetch `shas` from `origin` in a single negotiation.
pub fn fetch_commits(repo: &Path, shas: &[&str]) -> AppResult<()> {
    for sha in shas {
        validate_commit_ref(sha)?;
    }
    let mut args = vec!["fetch", "--quiet", "--no-tags", "origin"];
    args.extend_from_slice(shas);
    run_git(repo, &args).map(|_| ()).map_err(|e| {
        AppError::Git(format!(
            "commits {} are not in the local clone and could not be fetched from origin: {e}",
            shas.join(", ")
        ))
    })
}

/// Make sure every one of `shas` is present as a local object, fetching the missing ones from
/// `origin` in one go. GitHub serves any commit reachable from a PR (fork PRs included) this way.
pub fn ensure_commits(repo: &Path, shas: &[&str]) -> AppResult<()> {
    for sha in shas {
        validate_commit_ref(sha)?;
    }
    let missing: Vec<&str> = shas.iter().copied().filter(|sha| !has_commit(repo, sha)).collect();
    if missing.is_empty() {
        return Ok(());
    }
    fetch_commits(repo, &missing)
}

/// Make sure `sha` is present as a local object, fetching it from `origin` if not.
pub fn ensure_commit(repo: &Path, sha: &str) -> AppResult<()> {
    ensure_commits(repo, &[sha])
}

static BG_FETCHES: std::sync::Mutex<Option<std::collections::HashSet<std::path::PathBuf>>> =
    std::sync::Mutex::new(None);

/// Fetch `shas` on a background thread, so a diff served from the GitHub API doesn't wait for the
/// objects (a PR with big screenshots is hundreds of MB). One fetch per clone at a time: a repeat
/// request while one runs is dropped, the next open finds the commits missing and asks again.
pub fn fetch_commits_in_background(repo: &Path, shas: &[&str]) {
    let repo = repo.to_path_buf();
    if !BG_FETCHES.lock().unwrap().get_or_insert_with(Default::default).insert(repo.clone()) {
        return;
    }
    let shas: Vec<String> = shas.iter().map(|s| s.to_string()).collect();
    std::thread::spawn(move || {
        let refs: Vec<&str> = shas.iter().map(String::as_str).collect();
        match fetch_commits(&repo, &refs) {
            Ok(()) => log::info!(target: "git", "background fetch of {} commit(s) done in {}", refs.len(), repo.display()),
            Err(e) => log::warn!(target: "git", "background fetch failed in {}: {e}", repo.display()),
        }
        if let Some(set) = BG_FETCHES.lock().unwrap().as_mut() {
            set.remove(&repo);
        }
    });
}

pub fn rev_parse(path: &Path, rev: &str) -> AppResult<String> {
    validate_commit_ref(rev)?;
    Ok(run_git(
        path,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{rev}^{{commit}}"),
        ],
    )?
    .trim()
    .to_string())
}

/// First merge-base of two revs (`git merge-base a b`).
pub fn merge_base(repo: &Path, a: &str, b: &str) -> AppResult<String> {
    let a = rev_parse(repo, a)?;
    let b = rev_parse(repo, b)?;
    Ok(run_git(repo, &["merge-base", &a, &b])?.trim().to_string())
}

/// Full contents of `file_path` as of `sha` (`git show <sha>:<file_path>`),
/// used to reveal collapsed context lines around a diff.
pub fn show_file(path: &Path, sha: &str, file_path: &str) -> AppResult<String> {
    let sha = rev_parse(path, sha)?;
    run_git(
        path,
        &[
            "show",
            "--no-ext-diff",
            "--no-textconv",
            &format!("{sha}:{file_path}"),
            "--",
        ],
    )
}

/// `git grep -nE <pattern>` over the tree at `sha`, limited to the `globs` pathspecs. Returns
/// `(path, 1-based line, line text)`; no match is an empty list, not an error.
pub fn grep_at(path: &Path, sha: &str, pattern: &str, globs: &[&str]) -> AppResult<Vec<(String, u32, String)>> {
    let sha = rev_parse(path, sha)?;
    let mut args = vec!["grep", "-n", "-I", "-E", "--no-color", "-e", pattern, &sha, "--"];
    args.extend(globs);
    let out = match run_git(path, &args) {
        Ok(out) => out,
        // exit 1 with no stderr is how `git grep` says "no matches"
        Err(AppError::Git(msg)) if msg.is_empty() => return Ok(vec![]),
        Err(e) => return Err(e),
    };
    let prefix = format!("{sha}:");
    Ok(out
        .lines()
        .filter_map(|l| {
            let mut it = l.strip_prefix(&prefix)?.splitn(3, ':');
            let file = it.next()?.to_string();
            let line = it.next()?.parse().ok()?;
            Some((file, line, it.next().unwrap_or("").to_string()))
        })
        .collect())
}

/// `git grep -lE` over the tree at `sha` (already a full commit id): the paths (within `globs`)
/// with at least one match for any of `patterns`. No match is an empty list, not an error.
pub fn grep_files_at(path: &Path, sha: &str, patterns: &[String], globs: &[&str]) -> AppResult<Vec<String>> {
    let mut args = vec!["grep", "-l", "-I", "-E", "--no-color"];
    for p in patterns {
        args.extend(["-e", p.as_str()]);
    }
    args.extend([sha, "--"]);
    args.extend(globs);
    let out = match run_git(path, &args) {
        Ok(out) => out,
        Err(AppError::Git(msg)) if msg.is_empty() => return Ok(vec![]),
        Err(e) => return Err(e),
    };
    let prefix = format!("{sha}:");
    Ok(out.lines().filter_map(|l| l.strip_prefix(&prefix).map(str::to_string)).collect())
}


/// Contents of `paths` at `sha` (a full commit id), read through one `git cat-file --batch`
/// process. Files that do not exist or are larger than `max_bytes` come back as `None` (still in
/// input order).
pub fn cat_files(path: &Path, sha: &str, paths: &[String], max_bytes: usize) -> AppResult<Vec<Option<String>>> {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::Stdio;

    let mut child = Command::new(crate::tools::git_bin())
        .env("GIT_OPTIONAL_LOCKS", "0")
        .arg("-C")
        .arg(path)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| AppError::Git(format!("failed to spawn git: {e}")))?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let requests: Vec<String> = paths.iter().map(|p| format!("{sha}:{p}\n")).collect();
    // Written from a thread: a large batch would otherwise fill the pipe while we are not reading.
    let writer = std::thread::spawn(move || {
        for r in requests {
            if stdin.write_all(r.as_bytes()).is_err() {
                break;
            }
        }
    });
    let mut out = BufReader::new(child.stdout.take().expect("piped stdout"));
    let mut files = Vec::with_capacity(paths.len());
    let mut header = String::new();
    for _ in paths {
        header.clear();
        if out.read_line(&mut header).map_err(|e| AppError::Git(e.to_string()))? == 0 {
            files.push(None);
            continue;
        }
        // `<oid> <type> <size>` or `<spec> missing`
        let size = match header.trim_end().rsplit(' ').next().zip(header.split(' ').nth(1)) {
            Some((size, "blob")) => size.parse::<usize>().ok(),
            _ => None,
        };
        let Some(size) = size else {
            files.push(None);
            continue;
        };
        let mut buf = vec![0u8; size + 1]; // content + trailing newline
        out.read_exact(&mut buf).map_err(|e| AppError::Git(e.to_string()))?;
        buf.truncate(size);
        files.push(if size > max_bytes { None } else { Some(String::from_utf8_lossy(&buf).into_owned()) });
    }
    drop(out);
    let _ = writer.join();
    let _ = child.wait();
    Ok(files)
}

/// Unified diff between two refs. `three_dot` uses the merge-base (GitHub PR
/// semantics); otherwise a plain two-dot diff.
pub fn diff(path: &Path, base: &str, head: &str, three_dot: bool) -> AppResult<String> {
    let base = rev_parse(path, base)?;
    let head = rev_parse(path, head)?;
    let range = if three_dot {
        format!("{base}...{head}")
    } else {
        format!("{base}..{head}")
    };
    run_git(
        path,
        &[
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            &range,
            "--",
        ],
    )
}

/// Plain two-dot diff between two commits (literal line evolution old→new).
/// Two-dot, NOT three-dot: merge-base semantics are irrelevant here. Kept
/// alongside the path-scoped variant the re-anchor helper uses (Spec 01 §2).
#[allow(dead_code)]
pub fn diff_shas(repo: &Path, old_sha: &str, new_sha: &str) -> AppResult<String> {
    diff(repo, old_sha, new_sha, false)
}

/// The subset of `paths` that `.gitattributes` marks `linguist-generated`.
pub fn linguist_generated(repo: &Path, paths: &[&str]) -> AppResult<Vec<String>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let mut args = vec!["check-attr", "-z", "linguist-generated", "--"];
    args.extend_from_slice(paths);
    let out = run_git(repo, &args)?;
    let fields: Vec<&str> = out.split('\0').collect();
    Ok(fields
        .chunks(3)
        .filter(|c| c.len() == 3 && matches!(c[2], "set" | "true"))
        .map(|c| c[0].to_string())
        .collect())
}

/// Two-dot diff scoped to a single file, so the diff parser only sees one file.
pub fn diff_shas_path(
    repo: &Path,
    old_sha: &str,
    new_sha: &str,
    file_path: &str,
) -> AppResult<String> {
    let old_sha = rev_parse(repo, old_sha)?;
    let new_sha = rev_parse(repo, new_sha)?;
    run_git(
        repo,
        &[
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            &format!("{old_sha}..{new_sha}"),
            "--",
            file_path,
        ],
    )
}

/// What the "since" side of a rebase-aware interdiff is. A PR that was
/// rebased since the earlier pin can't just be diffed `old_head..new_head`:
/// that also shows every upstream commit the rebase picked up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterdiffBase {
    /// Same merge-base as before: the earlier head is the since side.
    Same(String),
    /// The earlier PR changes replayed onto the new merge-base: a tree OID.
    Replayed(String),
    /// The replay conflicted or could not run: the earlier head, with the diff
    /// limited to files either PR version touches. Approximate.
    Approximate(String),
}

impl InterdiffBase {
    /// The tree-ish the diff starts from, also the LEFT side's file source.
    pub fn treeish(&self) -> &str {
        match self {
            Self::Same(s) | Self::Replayed(s) | Self::Approximate(s) => s,
        }
    }
}

/// `git merge-tree --write-tree`: the tree of `new_base` plus what the earlier
/// PR (`old_base..old_head`) changed. None on conflicts, or when git can't do it
/// (needs a clone and git >= 2.40). Writes loose objects, never a ref.
fn replay_tree(repo: &Path, old_base: &str, old_head: &str, new_base: &str) -> Option<String> {
    let output = Command::new(crate::tools::git_bin())
        .env("GIT_OPTIONAL_LOCKS", "0")
        .arg("-C")
        .arg(repo)
        .args(["merge-tree", "--write-tree", "--no-messages"])
        .arg(format!("--merge-base={old_base}"))
        .arg(new_base)
        .arg(old_head)
        .output()
        .ok()?;
    // Exit 1 = conflicts: the tree is then not a faithful replay.
    if !output.status.success() {
        return None;
    }
    let out = String::from_utf8_lossy(&output.stdout);
    let tree = out.lines().next()?.trim();
    (!tree.is_empty()).then(|| tree.to_string())
}

/// Resolve the since side of an interdiff between an earlier pin
/// (`old_base`, `old_head`) and the new merge-base.
pub fn interdiff_base(repo: &Path, old_base: &str, old_head: &str, new_base: &str) -> AppResult<InterdiffBase> {
    ensure_commits(repo, &[old_base, old_head, new_base])?;
    let old_base = rev_parse(repo, old_base)?;
    let old_head = rev_parse(repo, old_head)?;
    let new_base = rev_parse(repo, new_base)?;
    if old_base == new_base {
        return Ok(InterdiffBase::Same(old_head));
    }
    Ok(match replay_tree(repo, &old_base, &old_head, &new_base) {
        Some(tree) => InterdiffBase::Replayed(tree),
        None => InterdiffBase::Approximate(old_head),
    })
}

fn changed_paths(repo: &Path, a: &str, b: &str) -> AppResult<Vec<String>> {
    let out = run_git(repo, &["diff", "--name-only", "-z", "--no-color", a, b, "--"])?;
    Ok(out.split('\0').filter(|p| !p.is_empty()).map(str::to_string).collect())
}

/// What changed between an earlier PR pin and `new_head`, rebase-aware: the
/// diff of the since side ([`interdiff_base`]) against `new_head`.
pub fn interdiff(
    repo: &Path,
    old_base: &str,
    old_head: &str,
    new_base: &str,
    new_head: &str,
) -> AppResult<String> {
    ensure_commits(repo, &[new_head, old_base, old_head, new_base])?;
    let new_head = rev_parse(repo, new_head)?;
    let base = interdiff_base(repo, old_base, old_head, new_base)?;
    let mut args: Vec<String> = ["diff", "--no-color", "--no-ext-diff", "--no-textconv"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    args.push(base.treeish().to_string());
    args.push(new_head.clone());
    args.push("--".into());
    if matches!(base, InterdiffBase::Approximate(_)) {
        let mut paths = changed_paths(repo, old_base, old_head)?;
        paths.extend(changed_paths(repo, new_base, &new_head)?);
        paths.sort();
        paths.dedup();
        if paths.is_empty() {
            return Ok(String::new());
        }
        args.extend(paths);
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    run_git(repo, &args)
}

/// Contents of `file_path` in any tree-ish (a replayed tree has no commit, so
/// [`show_file`]'s commit check would reject it).
pub fn show_blob(repo: &Path, treeish: &str, file_path: &str) -> AppResult<String> {
    validate_commit_ref(treeish)?;
    run_git(
        repo,
        &["show", "--no-ext-diff", "--no-textconv", &format!("{treeish}:{file_path}"), "--"],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    // ---- pure parsing -----------------------------------------------------

    #[test]
    fn parse_owner_repo_ssh() {
        let r = parse_owner_repo("git@github.com:owner/repo.git");
        assert_eq!(r.owner.as_deref(), Some("owner"));
        assert_eq!(r.name.as_deref(), Some("repo"));
    }

    #[test]
    fn parse_owner_repo_https() {
        let r = parse_owner_repo("https://github.com/some-org/some-repo.git");
        assert_eq!(r.owner.as_deref(), Some("some-org"));
        assert_eq!(r.name.as_deref(), Some("some-repo"));
    }

    #[test]
    fn parse_owner_repo_https_without_git_suffix() {
        let r = parse_owner_repo("https://github.com/a/b");
        assert_eq!(r.owner.as_deref(), Some("a"));
        assert_eq!(r.name.as_deref(), Some("b"));
    }

    #[test]
    fn parse_owner_repo_trims_whitespace_and_trailing_newline() {
        let r = parse_owner_repo("  git@github.com:owner/repo.git\n");
        assert_eq!(r.owner.as_deref(), Some("owner"));
        assert_eq!(r.name.as_deref(), Some("repo"));
    }

    #[test]
    fn parse_owner_repo_non_github_falls_back_to_last_two_segments() {
        let r = parse_owner_repo("https://gitlab.com/group/project.git");
        assert_eq!(r.owner.as_deref(), Some("group"));
        assert_eq!(r.name.as_deref(), Some("project"));
    }

    #[test]
    fn parse_owner_repo_garbage_yields_none() {
        let r = parse_owner_repo("not-a-url");
        assert!(r.owner.is_none());
        assert!(r.name.is_none());
    }

    // ---- integration against a real temporary git repo --------------------

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("spawn git");
        assert!(
            status.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&status.stderr)
        );
    }

    /// Build a repo with a `main` branch (one commit) and a `feature` branch
    /// that adds a line, so diffs/branches are deterministic.
    fn fixture_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        git(p, &["config", "user.email", "test@example.com"]);
        git(p, &["config", "user.name", "Test"]);
        fs::write(p.join("file.txt"), "line1\nline2\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "initial"]);

        git(p, &["checkout", "-q", "-b", "feature"]);
        fs::write(p.join("file.txt"), "line1\nline2\nline3\n").unwrap();
        git(p, &["commit", "-q", "-am", "add line3"]);
        git(p, &["checkout", "-q", "main"]);
        dir
    }

    #[test]
    fn linguist_generated_reads_gitattributes() {
        let dir = fixture_repo();
        let p = dir.path();
        fs::write(p.join(".gitattributes"), "gen/** linguist-generated\nx.pb.go linguist-generated=true\n")
            .unwrap();
        let got = linguist_generated(p, &["gen/a.ts", "x.pb.go", "file.txt"]).unwrap();
        assert_eq!(got, ["gen/a.ts", "x.pb.go"]);
        assert!(linguist_generated(p, &[]).unwrap().is_empty());
    }

    /// Build a repo with two linear commits on `main`. H1 has a few lines; H2
    /// inserts a line above an existing one, so the SHA-to-SHA diff is
    /// deterministic. Returns `(dir, h1, h2)`.
    fn fixture_repo_two_heads() -> (TempDir, String, String) {
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        git(p, &["config", "user.email", "test@example.com"]);
        git(p, &["config", "user.name", "Test"]);
        fs::write(p.join("file.txt"), "alpha\nbeta\ngamma\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "h1"]);
        let h1 = rev_parse(p, "HEAD").unwrap();

        fs::write(p.join("file.txt"), "alpha\nINSERTED\nbeta\ngamma\n").unwrap();
        git(p, &["commit", "-q", "-am", "h2"]);
        let h2 = rev_parse(p, "HEAD").unwrap();
        (dir, h1, h2)
    }

    #[test]
    fn is_git_repo_true_for_repo_false_otherwise() {
        let repo = fixture_repo();
        assert!(is_git_repo(repo.path()));

        let empty = TempDir::new().unwrap();
        assert!(!is_git_repo(empty.path()));
    }

    #[test]
    fn rev_parse_returns_a_sha() {
        let repo = fixture_repo();
        let sha = rev_parse(repo.path(), "main").unwrap();
        assert_eq!(sha.len(), 40, "expected full sha, got {sha:?}");
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn commit_refs_reject_options_ranges_and_non_commits() {
        let repo = fixture_repo();
        for rev in [
            "",
            "--all",
            "--output=/tmp/unused",
            "main\nfeature",
            "main..feature",
            "main:file.txt",
        ] {
            assert!(rev_parse(repo.path(), rev).is_err(), "accepted {rev:?}");
        }
        git(
            repo.path(),
            &["tag", "-a", "v1", "-m", "release", "feature"],
        );
        assert_eq!(
            rev_parse(repo.path(), "v1").unwrap(),
            rev_parse(repo.path(), "feature").unwrap()
        );
        assert_eq!(
            rev_parse(repo.path(), "feature~1").unwrap(),
            rev_parse(repo.path(), "main").unwrap()
        );
    }

    /// Clone `fixture_repo()` so the clone has an `origin` remote, then advance
    /// the origin with a new commit the clone has never fetched. Returns
    /// `(origin_dir, clone_dir, missing_sha)` — the caller must keep `origin_dir`
    /// alive for as long as `clone_dir`'s remote needs to be reachable.
    fn fixture_clone_missing_commit() -> (TempDir, TempDir, String) {
        let origin = fixture_repo();
        let clone = TempDir::new().unwrap();
        git(
            clone.path(),
            &[
                "clone",
                "-q",
                origin.path().to_str().unwrap(),
                clone.path().to_str().unwrap(),
            ],
        );

        git(origin.path(), &["checkout", "-q", "main"]);
        fs::write(origin.path().join("file.txt"), "line1\nline2\nextra\n").unwrap();
        git(origin.path(), &["commit", "-q", "-am", "advance origin"]);
        let missing_sha = rev_parse(origin.path(), "HEAD").unwrap();

        (origin, clone, missing_sha)
    }

    #[test]
    fn ensure_commit_fetches_a_missing_sha() {
        let (_origin, clone, missing_sha) = fixture_clone_missing_commit();
        assert!(rev_parse(clone.path(), &missing_sha).is_err());

        ensure_commit(clone.path(), &missing_sha).unwrap();
        assert_eq!(rev_parse(clone.path(), &missing_sha).unwrap(), missing_sha);
    }

    #[test]
    fn has_commit_tells_present_from_missing_and_ensure_commits_fetches_in_one_go() {
        let (_origin, clone, missing_sha) = fixture_clone_missing_commit();
        assert!(!has_commit(clone.path(), &missing_sha));
        assert!(!has_commit(clone.path(), "--all"));

        ensure_commits(clone.path(), &[&missing_sha]).unwrap();
        assert!(has_commit(clone.path(), &missing_sha));
        // Nothing missing: no fetch needed, still ok.
        ensure_commits(clone.path(), &[&missing_sha]).unwrap();
    }

    #[test]
    fn ensure_commit_is_a_no_op_for_a_present_sha() {
        let repo = fixture_repo();
        let sha = rev_parse(repo.path(), "main").unwrap();
        ensure_commit(repo.path(), &sha).unwrap();
    }

    #[test]
    fn ensure_commit_rejects_invalid_refs() {
        let repo = fixture_repo();
        assert!(ensure_commit(repo.path(), "--all").is_err());
    }

    #[test]
    fn crafted_diff_refs_cannot_overwrite_files() {
        let repo = fixture_repo();
        let sentinel = repo.path().join("sentinel.txt");
        fs::write(&sentinel, "preserve me").unwrap();
        fs::write(repo.path().join("file.txt"), "modified\n").unwrap();
        let destination = sentinel.to_str().unwrap();
        for three_dot in [false, true] {
            assert!(diff(repo.path(), "--output=/", destination, three_dot).is_err());
            assert!(diff(repo.path(), "main", "--output=/", three_dot).is_err());
        }
        assert!(diff_shas(repo.path(), "--output=/", destination).is_err());
        assert!(diff_shas_path(repo.path(), "--output=/", destination, "file.txt").is_err());
        assert_eq!(fs::read_to_string(sentinel).unwrap(), "preserve me");
    }

    #[test]
    fn list_branches_includes_local_heads() {
        let repo = fixture_repo();
        let names: Vec<String> = list_branches(repo.path())
            .unwrap()
            .into_iter()
            .map(|b| b.name)
            .collect();
        assert!(names.contains(&"main".to_string()), "got {names:?}");
        assert!(names.contains(&"feature".to_string()), "got {names:?}");
        // No symbolic HEAD pointers leak through.
        assert!(!names.iter().any(|n| n.ends_with("/HEAD")));
    }

    #[test]
    fn diff_shows_added_line() {
        let repo = fixture_repo();
        let out = diff(repo.path(), "main", "feature", false).unwrap();
        assert!(out.contains("+line3"), "diff was: {out}");
        assert!(out.contains("file.txt"));
    }

    #[test]
    fn diff_empty_when_no_changes() {
        let repo = fixture_repo();
        let out = diff(repo.path(), "main", "main", false).unwrap();
        assert!(out.trim().is_empty(), "expected empty diff, got: {out}");
    }

    #[test]
    fn run_git_surfaces_error_on_bad_ref() {
        let repo = fixture_repo();
        let err = rev_parse(repo.path(), "no-such-ref").unwrap_err();
        assert!(matches!(err, AppError::Git(_)));
    }

    #[test]
    fn show_file_returns_contents_at_ref() {
        let repo = fixture_repo();
        let out = show_file(repo.path(), "main", "file.txt").unwrap();
        assert_eq!(out, "line1\nline2\n");

        let feature = show_file(repo.path(), "feature", "file.txt").unwrap();
        assert_eq!(feature, "line1\nline2\nline3\n");
    }

    #[test]
    fn show_file_errors_on_missing_path() {
        let repo = fixture_repo();
        let err = show_file(repo.path(), "main", "no-such-file.txt").unwrap_err();
        assert!(matches!(err, AppError::Git(_)));
    }

    #[test]
    fn diff_shas_shows_inserted_line() {
        let (repo, h1, h2) = fixture_repo_two_heads();
        let out = diff_shas(repo.path(), &h1, &h2).unwrap();
        assert!(out.contains("+INSERTED"), "diff was: {out}");
        assert!(out.contains("file.txt"));
    }

    #[test]
    fn diff_shas_path_scopes_to_one_file() {
        let (repo, h1, h2) = fixture_repo_two_heads();
        let out = diff_shas_path(repo.path(), &h1, &h2, "file.txt").unwrap();
        assert!(out.contains("+INSERTED"), "diff was: {out}");
        assert!(out.contains("file.txt"));
    }

    #[test]
    fn merge_base_of_diverged_branches() {
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        git(p, &["config", "user.email", "test@example.com"]);
        git(p, &["config", "user.name", "Test"]);
        fs::write(p.join("file.txt"), "a\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "fork point"]);
        let fork = rev_parse(p, "HEAD").unwrap();

        // Branch `feat` off the fork point, then advance both branches so they
        // diverge — the merge-base must remain the fork commit.
        git(p, &["checkout", "-q", "-b", "feat"]);
        fs::write(p.join("file.txt"), "a\nfeat\n").unwrap();
        git(p, &["commit", "-q", "-am", "feat work"]);

        git(p, &["checkout", "-q", "main"]);
        fs::write(p.join("file.txt"), "a\nmain\n").unwrap();
        git(p, &["commit", "-q", "-am", "main work"]);

        let mb = merge_base(p, "main", "feat").unwrap();
        assert_eq!(mb, fork);
        assert_eq!(mb, rev_parse(p, "main~1").unwrap());
    }

    // ---- interdiff ----------------------------------------------------------

    /// `main` with a 10-line `a.txt` and a `b.txt`; returns the repo.
    fn interdiff_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        git(p, &["config", "user.email", "test@example.com"]);
        git(p, &["config", "user.name", "Test"]);
        let a: String = (1..=10).map(|i| format!("line{i}\n")).collect();
        fs::write(p.join("a.txt"), a).unwrap();
        fs::write(p.join("b.txt"), "b\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "base"]);
        dir
    }

    fn sed_line(p: &Path, file: &str, n: usize, text: &str) {
        let src = fs::read_to_string(p.join(file)).unwrap();
        let out: String = src
            .lines()
            .enumerate()
            .map(|(i, l)| format!("{}\n", if i + 1 == n { text } else { l }))
            .collect();
        fs::write(p.join(file), out).unwrap();
    }

    fn rev(p: &Path, r: &str) -> String {
        rev_parse(p, r).unwrap()
    }

    #[test]
    fn interdiff_without_a_rebase_is_the_plain_head_diff() {
        let dir = interdiff_repo();
        let p = dir.path();
        let base = rev(p, "main");
        git(p, &["checkout", "-q", "-b", "pr"]);
        sed_line(p, "a.txt", 2, "pr1");
        git(p, &["commit", "-q", "-am", "pr first"]);
        let old_head = rev(p, "HEAD");
        sed_line(p, "a.txt", 9, "fix");
        git(p, &["commit", "-q", "-am", "pr second"]);
        let new_head = rev(p, "HEAD");

        let out = interdiff(p, &base, &old_head, &base, &new_head).unwrap();
        assert_eq!(out, diff(p, &old_head, &new_head, false).unwrap());
        assert!(out.contains("+fix") && !out.contains("pr1"));
    }

    #[test]
    fn interdiff_hides_upstream_changes_picked_up_by_a_rebase() {
        let dir = interdiff_repo();
        let p = dir.path();
        let old_base = rev(p, "main");
        git(p, &["checkout", "-q", "-b", "pr"]);
        sed_line(p, "a.txt", 2, "pr1");
        git(p, &["commit", "-q", "-am", "pr first"]);
        let old_head = rev(p, "HEAD");

        // Upstream moves on (touching another file and another part of a.txt).
        git(p, &["checkout", "-q", "main"]);
        fs::write(p.join("b.txt"), "b upstream\n").unwrap();
        sed_line(p, "a.txt", 5, "upstream5");
        git(p, &["commit", "-q", "-am", "upstream"]);
        let new_base = rev(p, "main");

        // The author rebases onto it and adds one more fix.
        git(p, &["checkout", "-q", "-b", "pr2"]);
        sed_line(p, "a.txt", 2, "pr1");
        git(p, &["commit", "-q", "-am", "pr first, rebased"]);
        sed_line(p, "a.txt", 9, "fix");
        git(p, &["commit", "-q", "-am", "pr second"]);
        let new_head = rev(p, "HEAD");

        let out = interdiff(p, &old_base, &old_head, &new_base, &new_head).unwrap();
        assert!(out.contains("+fix"), "{out}");
        let changed: Vec<&str> = out
            .lines()
            .filter(|l| (l.starts_with('+') || l.starts_with('-')) && !l.starts_with("+++") && !l.starts_with("---"))
            .collect();
        assert_eq!(changed, ["-line9", "+fix"], "{out}");
        assert!(!out.contains("b.txt"), "{out}");
        assert!(matches!(
            interdiff_base(p, &old_base, &old_head, &new_base).unwrap(),
            InterdiffBase::Replayed(_)
        ));
        // The replayed tree is the LEFT side: it already has the rebased pr1 line.
        let tree = interdiff_base(p, &old_base, &old_head, &new_base).unwrap();
        let left = show_blob(p, tree.treeish(), "a.txt").unwrap();
        assert!(left.contains("pr1") && left.contains("upstream5") && !left.contains("fix"));
    }

    #[test]
    fn interdiff_falls_back_when_the_replay_conflicts() {
        let dir = interdiff_repo();
        let p = dir.path();
        let old_base = rev(p, "main");
        git(p, &["checkout", "-q", "-b", "pr"]);
        sed_line(p, "a.txt", 2, "pr1");
        git(p, &["commit", "-q", "-am", "pr first"]);
        let old_head = rev(p, "HEAD");

        git(p, &["checkout", "-q", "main"]);
        sed_line(p, "a.txt", 2, "upstream2");
        fs::write(p.join("b.txt"), "b upstream\n").unwrap();
        git(p, &["commit", "-q", "-am", "upstream"]);
        let new_base = rev(p, "main");

        git(p, &["checkout", "-q", "-b", "pr2"]);
        sed_line(p, "a.txt", 2, "resolved");
        git(p, &["commit", "-q", "-am", "pr rebased, conflict resolved"]);
        let new_head = rev(p, "HEAD");

        assert!(matches!(
            interdiff_base(p, &old_base, &old_head, &new_base).unwrap(),
            InterdiffBase::Approximate(_)
        ));
        let out = interdiff(p, &old_base, &old_head, &new_base, &new_head).unwrap();
        assert!(out.contains("-pr1") && out.contains("+resolved"), "{out}");
        assert!(!out.contains("b.txt"), "limited to files the PR touches: {out}");
    }
}
