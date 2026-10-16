//! Lazy, refcounted watchers on repositories' git dirs (ADR 0004). A view
//! acquires a repo while it is open; the watcher stops `linger` after the last
//! release, so quick view switches don't churn it.

use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notify_debouncer_mini::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, Debouncer};
use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::git::run_git;

const DEBOUNCE: Duration = Duration::from_millis(300);
pub const LINGER: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    /// A checkout moved (HEAD of the main checkout or a linked worktree).
    Head,
    /// Branches or tags moved.
    Refs,
    /// Something was staged / unstaged.
    Index,
    /// A `git fetch` completed.
    Fetch,
}

/// What a changed path under the git dir means, or None for noise (objects, logs, …).
pub fn classify(git_dir: &Path, path: &Path) -> Option<ChangeKind> {
    let rel = path.strip_prefix(git_dir).ok()?;
    let parts: Vec<&str> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    let leaf = |name: &str| name.strip_suffix(".lock").unwrap_or(name).to_string();
    match parts.as_slice() {
        [name] => match leaf(name).as_str() {
            "HEAD" => Some(ChangeKind::Head),
            "index" => Some(ChangeKind::Index),
            "FETCH_HEAD" => Some(ChangeKind::Fetch),
            "packed-refs" => Some(ChangeKind::Refs),
            _ => None,
        },
        ["refs", ..] => Some(ChangeKind::Refs),
        ["worktrees", _, name] => match leaf(name).as_str() {
            "HEAD" => Some(ChangeKind::Head),
            "index" => Some(ChangeKind::Index),
            _ => None,
        },
        _ => None,
    }
}

/// The repository's common git dir (shared by all its worktrees), with
/// symlinks resolved so it prefixes the paths the OS reports.
pub fn git_common_dir(checkout: &Path) -> AppResult<PathBuf> {
    let out = run_git(checkout, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    Ok(std::fs::canonicalize(out.trim())?)
}

pub type OnChange = Arc<dyn Fn(i64, Vec<ChangeKind>) + Send + Sync>;

struct Entry {
    refs: usize,
    /// Bumped on every acquire, so a pending delayed stop can tell it's stale.
    generation: u64,
    _debouncer: Debouncer<RecommendedWatcher>,
}

pub struct Watchers {
    entries: Arc<Mutex<HashMap<i64, Entry>>>,
    linger: Duration,
}

impl Default for Watchers {
    fn default() -> Self {
        Self::new(LINGER)
    }
}

impl Watchers {
    pub fn new(linger: Duration) -> Self {
        Self { entries: Arc::default(), linger }
    }

    /// Adds a holder for `repo_id`, starting a watcher on `checkout`'s git dir
    /// if none is running.
    pub fn acquire(&self, repo_id: i64, checkout: &Path, on_change: OnChange) -> AppResult<()> {
        let mut entries = self.entries.lock().unwrap();
        if let Some(entry) = entries.get_mut(&repo_id) {
            entry.refs += 1;
            entry.generation += 1;
            return Ok(());
        }
        let git_dir = git_common_dir(checkout)?;
        let watched = git_dir.clone();
        let mut debouncer = new_debouncer(DEBOUNCE, move |res: DebounceEventResult| {
            let Ok(events) = res else { return };
            let kinds: BTreeSet<ChangeKind> =
                events.iter().filter_map(|e| classify(&watched, &e.path)).collect();
            if !kinds.is_empty() {
                on_change(repo_id, kinds.into_iter().collect());
            }
        })
        .map_err(|e| AppError::Other(format!("could not start watcher: {e}")))?;
        debouncer
            .watcher()
            .watch(&git_dir, RecursiveMode::Recursive)
            .map_err(|e| AppError::Other(format!("could not watch {}: {e}", git_dir.display())))?;
        log::info!("[watch] start repo {repo_id} ({})", git_dir.display());
        entries.insert(repo_id, Entry { refs: 1, generation: 0, _debouncer: debouncer });
        Ok(())
    }

    /// Drops a holder. The watcher stops `linger` later unless re-acquired.
    pub fn release(&self, repo_id: i64) {
        let generation = {
            let mut entries = self.entries.lock().unwrap();
            let Some(entry) = entries.get_mut(&repo_id) else { return };
            entry.refs = entry.refs.saturating_sub(1);
            if entry.refs > 0 {
                return;
            }
            entry.generation
        };
        let entries = self.entries.clone();
        let linger = self.linger;
        std::thread::spawn(move || {
            std::thread::sleep(linger);
            let mut entries = entries.lock().unwrap();
            if entries.get(&repo_id).is_some_and(|e| e.refs == 0 && e.generation == generation) {
                entries.remove(&repo_id);
                log::info!("[watch] stop repo {repo_id}");
            }
        });
    }

    pub fn is_watching(&self, repo_id: i64) -> bool {
        self.entries.lock().unwrap().contains_key(&repo_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::sync::mpsc;
    use tempfile::TempDir;

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    #[test]
    fn classifies_git_dir_paths() {
        let g = Path::new("/r/.git");
        let k = |p: &str| classify(g, &g.join(p));
        assert_eq!(k("HEAD"), Some(ChangeKind::Head));
        assert_eq!(k("index.lock"), Some(ChangeKind::Index));
        assert_eq!(k("FETCH_HEAD"), Some(ChangeKind::Fetch));
        assert_eq!(k("packed-refs"), Some(ChangeKind::Refs));
        assert_eq!(k("refs/heads/feat/x"), Some(ChangeKind::Refs));
        assert_eq!(k("refs/remotes/origin/main.lock"), Some(ChangeKind::Refs));
        assert_eq!(k("worktrees/wt1/HEAD"), Some(ChangeKind::Head));
        assert_eq!(k("worktrees/wt1/index"), Some(ChangeKind::Index));
        assert_eq!(k("objects/ab/cdef"), None);
        assert_eq!(k("logs/HEAD"), None);
        assert_eq!(k("ORIG_HEAD"), None);
        assert_eq!(classify(g, Path::new("/elsewhere/HEAD")), None);
    }

    fn repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        git(dir.path(), &["-c", "user.email=t@e", "-c", "user.name=T", "commit", "-q", "--allow-empty", "-m", "c"]);
        dir
    }

    #[test]
    fn refcount_and_linger() {
        let dir = repo();
        let w = Watchers::new(Duration::from_millis(100));
        let noop: OnChange = Arc::new(|_, _| {});
        w.acquire(1, dir.path(), noop.clone()).unwrap();
        w.acquire(1, dir.path(), noop.clone()).unwrap();
        w.release(1);
        std::thread::sleep(Duration::from_millis(250));
        assert!(w.is_watching(1), "one holder left");
        w.release(1);
        w.acquire(1, dir.path(), noop).unwrap();
        std::thread::sleep(Duration::from_millis(250));
        assert!(w.is_watching(1), "re-acquired before linger elapsed");
        w.release(1);
        std::thread::sleep(Duration::from_millis(250));
        assert!(!w.is_watching(1));
    }

    #[test]
    fn reports_branch_creation() {
        let dir = repo();
        let w = Watchers::new(Duration::from_millis(100));
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        w.acquire(7, dir.path(), Arc::new(move |id, kinds| {
            let _ = tx.lock().unwrap().send((id, kinds));
        }))
        .unwrap();
        // FSEvents can take a moment to start delivering.
        std::thread::sleep(Duration::from_millis(300));
        git(dir.path(), &["branch", "feature"]);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let (id, kinds) = rx.recv_timeout(left).expect("no git-changed event");
            assert_eq!(id, 7);
            if kinds.contains(&ChangeKind::Refs) {
                break;
            }
        }
    }
}
