//! Runs image diff jobs in the background, one at a time app-wide, and decides when a review gets
//! one without being asked (a small set of changed images in a review with a local clone).

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use super::blob::{BlobSource, GitBlobs, Side};
use super::job::{self, MAX_PIXELS, PREVIEW_WIDTH};
use super::store::{self, Manifest, Progress, MANIFEST_VERSION};
use super::{changed_image_files, probe_dimensions, ImageFile};
use crate::commands::review::{diff_source, get_review_row, load_detail, review_since};
use crate::db::{Db, DbPath};
use crate::error::{AppError, AppResult};
use crate::git;

pub const EVENT: &str = "image-diff-updated";
/// A set bigger than this is left for a click.
pub const AUTO_MAX_FILES: usize = 20;
pub const AUTO_MAX_PIXELS: u64 = 25_000_000;
/// How long an auto run waits for the clone to finish fetching the review's commits.
const FETCH_WAIT: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Serialize)]
pub struct Updated {
    pub review_id: i64,
    /// `running`, `ready`, `failed`, or `none` (cancelled).
    pub state: &'static str,
    pub done: usize,
    pub total: usize,
    pub error: Option<String>,
    pub auto: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Request {
    review_id: i64,
    auto: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Enqueued {
    Duplicate,
    Queued,
    StartWorker,
}

#[derive(Debug, PartialEq, Eq)]
enum Cancelled {
    Running,
    Queued,
    NotActive,
}

#[derive(Default)]
struct Registry {
    current: Option<(i64, Arc<AtomicBool>)>,
    queue: VecDeque<Request>,
    worker: bool,
}

impl Registry {
    fn is_active(&self, review_id: i64) -> bool {
        self.current.as_ref().is_some_and(|(id, _)| *id == review_id) || self.queue.iter().any(|r| r.review_id == review_id)
    }

    fn enqueue(&mut self, req: Request) -> Enqueued {
        if self.is_active(req.review_id) {
            return Enqueued::Duplicate;
        }
        self.queue.push_back(req);
        if self.worker {
            Enqueued::Queued
        } else {
            self.worker = true;
            Enqueued::StartWorker
        }
    }

    /// The worker asks for its next job; `None` retires it.
    fn next(&mut self) -> Option<(Request, Arc<AtomicBool>)> {
        self.current = None;
        match self.queue.pop_front() {
            Some(req) => {
                let cancel = Arc::new(AtomicBool::new(false));
                self.current = Some((req.review_id, cancel.clone()));
                Some((req, cancel))
            }
            None => {
                self.worker = false;
                None
            }
        }
    }

    fn cancel(&mut self, review_id: i64) -> Cancelled {
        if let Some((id, flag)) = &self.current {
            if *id == review_id {
                flag.store(true, Ordering::Relaxed);
                return Cancelled::Running;
            }
        }
        let before = self.queue.len();
        self.queue.retain(|r| r.review_id != review_id);
        if self.queue.len() < before {
            Cancelled::Queued
        } else {
            Cancelled::NotActive
        }
    }
}

static REGISTRY: Mutex<Option<Registry>> = Mutex::new(None);
/// Reviews whose auto-run is being weighed, so concurrent diff loads weigh it once.
static PROBING: Mutex<Option<HashSet<i64>>> = Mutex::new(None);

fn registry<T>(f: impl FnOnce(&mut Registry) -> T) -> T {
    f(REGISTRY.lock().unwrap().get_or_insert_with(Registry::default))
}

pub fn is_active(review_id: i64) -> bool {
    registry(|r| r.is_active(review_id))
}

/// Stop a review's job (running or queued). A running one removes its own partial cache.
pub fn cancel(review_id: i64) -> bool {
    registry(|r| r.cancel(review_id)) != Cancelled::NotActive
}

/// The user stopped a job: a queued one is dropped here, a running one cleans up after itself.
pub fn cancel_job(app: &AppHandle, review_id: i64) {
    if registry(|r| r.cancel(review_id)) == Cancelled::Queued {
        store::remove_cache(&store::cache_dir(&db_path(app), review_id));
        emit(app, review_id, "none", 0, 0, None, false);
    }
}

/// What a review's cache looks like, as the panel and the menu need to know it.
#[derive(Debug, Serialize, PartialEq)]
pub struct Status {
    /// `none`, `running`, `ready`, `stale` (generated for an older pin), `interrupted` (the app
    /// quit mid-run) or `failed`.
    pub state: &'static str,
    pub done: usize,
    pub total: usize,
    pub error: Option<String>,
    pub generated_at: Option<String>,
    pub auto: bool,
    pub base_sha: Option<String>,
    pub head_sha: Option<String>,
    /// An AI report was written for these image diffs.
    pub has_report: bool,
    /// An AI report run is queued or running (filled by the command: it lives in the routines tables).
    pub report_running: bool,
}

impl Status {
    fn bare(state: &'static str) -> Self {
        Status {
            state,
            done: 0,
            total: 0,
            error: None,
            generated_at: None,
            auto: false,
            base_sha: None,
            head_sha: None,
            has_report: false,
            report_running: false,
        }
    }
}

pub fn status_of(dir: &Path, current_pin: &str, active: bool) -> Status {
    let progress = store::read_progress(dir);
    if active {
        let (done, total) = progress.map(|p| (p.done, p.total)).unwrap_or((0, 0));
        return Status { done, total, ..Status::bare("running") };
    }
    if let Some(m) = store::read_manifest(dir) {
        let skipped = m.files.iter().filter(|f| f.skipped.is_some()).count();
        return Status {
            total: m.files.len(),
            done: m.files.len() - skipped,
            generated_at: Some(m.generated_at),
            auto: m.auto,
            base_sha: m.base_sha,
            head_sha: m.head_sha,
            has_report: dir.join(store::REPORT_FINAL).exists(),
            ..Status::bare(if m.pin == current_pin { "ready" } else { "stale" })
        };
    }
    match progress {
        Some(p) if p.state == "failed" => Status { error: p.error, done: p.done, total: p.total, ..Status::bare("failed") },
        Some(p) => Status { done: p.done, total: p.total, ..Status::bare("interrupted") },
        None => Status::bare("none"),
    }
}

/// The review's pin as `store::pin_key` sees it.
pub fn current_pin(conn: &rusqlite::Connection, review_id: i64) -> AppResult<String> {
    let r = get_review_row(conn, review_id)?;
    Ok(store::pin_key(r.base_sha.as_deref(), r.head_sha.as_deref(), review_since(&r).as_ref().map(|(b, h)| (b.as_str(), h.as_str()))))
}

struct Inputs {
    files: Vec<ImageFile>,
    blobs: GitBlobs,
    pin: String,
    base_sha: Option<String>,
    head_sha: Option<String>,
}

/// The commits each side of the diff is read at. A "changes since" review compares from the
/// earlier head; a virtual PR's three-dot diff starts at the merge base.
fn side_revs(detail: &crate::db::models::ReviewDetail) -> (Option<String>, Option<String>) {
    let (r, t) = (&detail.review, &detail.target);
    let right = r.head_sha.clone().or_else(|| Some(t.head_ref.clone()));
    let left = if let Some((_, since_head)) = review_since(r) {
        Some(since_head)
    } else {
        let base = r.base_sha.clone().unwrap_or_else(|| t.base_ref.clone());
        let merge_base = (t.kind != "github_pr" && t.three_dot)
            .then(|| {
                let local = detail.local_path.as_deref()?;
                git::merge_base(Path::new(local), &base, right.as_deref()?).ok()
            })
            .flatten();
        Some(merge_base.unwrap_or(base))
    };
    (left, right)
}

fn gather(app: &AppHandle, review_id: i64) -> AppResult<Inputs> {
    let db = app.state::<Db>();
    let (detail, source) = {
        let conn = db.0.lock().unwrap();
        (load_detail(&conn, review_id)?, diff_source(&conn, review_id)?)
    };
    let diff = source.load()?;
    let r = &detail.review;
    let (left, right) = side_revs(&detail);
    let local = detail.local_path.as_ref().map(PathBuf::from).filter(|p| p.exists());
    let owner_name = detail.remote_owner.clone().zip(detail.remote_name.clone());
    Ok(Inputs {
        files: changed_image_files(&diff),
        pin: store::pin_key(r.base_sha.as_deref(), r.head_sha.as_deref(), review_since(r).as_ref().map(|(b, h)| (b.as_str(), h.as_str()))),
        base_sha: r.base_sha.clone(),
        head_sha: r.head_sha.clone(),
        blobs: GitBlobs { local, owner_name, left, right },
    })
}

fn db_path(app: &AppHandle) -> PathBuf {
    app.state::<DbPath>().0.clone()
}

fn emit(app: &AppHandle, review_id: i64, state: &'static str, done: usize, total: usize, error: Option<String>, auto: bool) {
    let _ = app.emit(EVENT, Updated { review_id, state, done, total, error, auto });
}

/// Wait for a background fetch of the clone (started by the diff load) so the blobs are local.
fn wait_for_fetch(local: &Path) {
    let started = std::time::Instant::now();
    while git::background_fetch_running(local) && started.elapsed() < FETCH_WAIT {
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn run(app: &AppHandle, req: &Request, cancel: &AtomicBool) {
    let id = req.review_id;
    let dir = store::cache_dir(&db_path(app), id);
    let fail = |msg: String| {
        log::warn!(target: "image_diff", "review {id}: {msg}");
        store::write_progress(&dir, &Progress { state: "failed".into(), done: 0, total: 0, error: Some(msg.clone()) });
        emit(app, id, "failed", 0, 0, Some(msg), req.auto);
    };
    let inputs = match gather(app, id) {
        Ok(i) => i,
        Err(e) => return fail(e.to_string()),
    };
    let total = inputs.files.len();
    store::remove_cache(&dir);
    store::write_progress(&dir, &Progress { state: "running".into(), done: 0, total, error: None });
    emit(app, id, "running", 0, total, None, req.auto);

    if let (Some(local), Some(left), Some(right)) = (&inputs.blobs.local, &inputs.blobs.left, &inputs.blobs.right) {
        wait_for_fetch(local);
        // Missing commits fall back to the GitHub API per blob, so a failed fetch isn't fatal.
        if let Err(e) = git::ensure_commits(local, &[left, right]) {
            log::debug!(target: "image_diff", "review {id}: {e}");
        }
    }
    let started = std::time::Instant::now();
    let result = job::generate(&inputs.blobs, &inputs.files, &dir, cancel, |done, total| {
        store::write_progress(&dir, &Progress { state: "running".into(), done, total, error: None });
        emit(app, id, "running", done, total, None, req.auto);
    });
    match result {
        Ok(files) => {
            let manifest = Manifest {
                version: MANIFEST_VERSION,
                review_id: id,
                pin: inputs.pin,
                base_sha: inputs.base_sha,
                head_sha: inputs.head_sha,
                generated_at: chrono::Utc::now().to_rfc3339(),
                preview_width: PREVIEW_WIDTH,
                auto: req.auto,
                files,
            };
            match store::write_json(&dir, store::MANIFEST, &manifest) {
                Ok(()) => {
                    let _ = std::fs::remove_file(dir.join(store::PROGRESS));
                    log::info!(target: "image_diff", "review {id}: {total} image(s) diffed in {}ms", started.elapsed().as_millis());
                    emit(app, id, "ready", total, total, None, req.auto);
                    if req.auto {
                        chain_report(app, id, &manifest.files);
                    }
                }
                Err(e) => fail(format!("writing the manifest failed: {e}")),
            }
        }
        Err(AppError::Other(m)) if m == "cancelled" => {
            store::remove_cache(&dir);
            emit(app, id, "none", 0, 0, None, req.auto);
        }
        Err(e) => fail(e.to_string()),
    }
}

/// Whether there is something for an AI report to describe: a compared pair with changed pixels.
pub fn has_changes_to_describe(files: &[store::FileEntry]) -> bool {
    files.iter().any(|f| f.skipped.is_none() && f.mismatch_px > 0)
}

/// After an automatic generation: write the AI report too, when AI is on, `claude` is there and
/// some pixels really changed. A failure here is only logged: the pictures are already usable.
fn chain_report(app: &AppHandle, review_id: i64, files: &[store::FileEntry]) {
    if !crate::ai_gate::is_enabled() || !has_changes_to_describe(files) {
        return;
    }
    let ctx = crate::routines::context::ContextRef { kind: "review".into(), id: review_id.to_string(), base_paths: vec![] };
    let launched = crate::commands::routines::builtin(&app.state::<Db>(), crate::routines::IMAGE_DIFF_REPORT_KEY)
        .and_then(|routine| crate::commands::routines::launch(app, &routine, &ctx, "", ""));
    if let Err(e) = launched {
        log::info!(target: "image_diff", "review {review_id}: no automatic AI report: {e}");
    }
}

fn worker(app: AppHandle) {
    while let Some((req, cancel)) = registry(Registry::next) {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&app, &req, &cancel)));
        if outcome.is_err() {
            let dir = store::cache_dir(&db_path(&app), req.review_id);
            let msg = "image diff generation crashed".to_string();
            store::write_progress(&dir, &Progress { state: "failed".into(), done: 0, total: 0, error: Some(msg.clone()) });
            emit(&app, req.review_id, "failed", 0, 0, Some(msg), req.auto);
        }
    }
}

/// Start (or regenerate) a review's image diffs in the background. A review that already has a
/// job queued or running is left alone. Returns whether a job was queued.
pub fn start(app: &AppHandle, review_id: i64, auto: bool) -> bool {
    if is_active(review_id) {
        return false;
    }
    let dir = store::cache_dir(&db_path(app), review_id);
    store::remove_cache(&dir);
    store::write_progress(&dir, &Progress { state: "running".into(), done: 0, total: 0, error: None });
    match registry(|r| r.enqueue(Request { review_id, auto })) {
        Enqueued::Duplicate => return false,
        Enqueued::Queued => {}
        Enqueued::StartWorker => {
            let app = app.clone();
            std::thread::spawn(move || worker(app));
        }
    }
    emit(app, review_id, "running", 0, 0, None, auto);
    true
}

/// Whether a set of images (the pixel counts of the larger side of each) is small enough to be
/// diffed without being asked.
pub fn within_budget(pixels: &[u64]) -> bool {
    pixels.len() <= AUTO_MAX_FILES && pixels.iter().sum::<u64>() <= AUTO_MAX_PIXELS && pixels.iter().all(|p| *p <= MAX_PIXELS)
}

fn dims(blobs: &GitBlobs, side: Side, path: &str) -> Option<u64> {
    let bytes = blobs.read(side, path).ok()?;
    probe_dimensions(&bytes, path).map(|(w, h)| w as u64 * h as u64)
}

/// A review's diff has just been loaded: if it changes a small set of images, has a local clone
/// and no up-to-date cache, generate them in the background. Runs on its own thread; the cheap
/// checks (any image at all?) run on the caller's.
pub fn maybe_auto(app: &AppHandle, review_id: i64, diff: Arc<String>) {
    if changed_image_files(&diff).is_empty() {
        return;
    }
    let fresh = PROBING.lock().unwrap().get_or_insert_with(HashSet::new).insert(review_id);
    if !fresh {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        if let Err(e) = auto(&app, review_id, &diff) {
            log::debug!(target: "image_diff", "review {review_id}: not auto-generated: {e}");
        }
        if let Some(set) = PROBING.lock().unwrap().as_mut() {
            set.remove(&review_id);
        }
    });
}

fn up_to_date(app: &AppHandle, review_id: i64) -> AppResult<bool> {
    let dir = store::cache_dir(&db_path(app), review_id);
    let pin = current_pin(&app.state::<Db>().0.lock().unwrap(), review_id)?;
    let s = status_of(&dir, &pin, is_active(review_id));
    // A failed or interrupted run is not retried on its own, only by a click.
    Ok(s.state != "stale" && s.state != "none")
}

fn auto(app: &AppHandle, review_id: i64, diff: &str) -> AppResult<()> {
    if up_to_date(app, review_id)? {
        return Err(AppError::Other("already generated, running or failed".into()));
    }
    let files = changed_image_files(diff);
    if files.len() > AUTO_MAX_FILES {
        return Err(AppError::Other(format!("{} images, over the {AUTO_MAX_FILES} limit", files.len())));
    }
    let detail = load_detail(&app.state::<Db>().0.lock().unwrap(), review_id)?;
    let local = detail
        .local_path
        .as_ref()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .ok_or_else(|| AppError::Other("no linked clone".into()))?;
    let (left, right) = side_revs(&detail);
    let (Some(left), Some(right)) = (left, right) else { return Err(AppError::Other("unresolved sides".into())) };
    wait_for_fetch(&local);
    git::ensure_commits(&local, &[&left, &right])?;
    // Local blobs only: a missing one must not turn into API downloads nobody asked for.
    let blobs = GitBlobs { local: Some(local), owner_name: None, left: Some(left), right: Some(right) };
    let mut pixels = Vec::with_capacity(files.len());
    for f in &files {
        let before = (f.change != super::store::Change::Added).then(|| dims(&blobs, Side::Left, f.left_path())).flatten();
        let after = (f.change != super::store::Change::Deleted).then(|| dims(&blobs, Side::Right, &f.path)).flatten();
        pixels.push(before.max(after).ok_or_else(|| AppError::Other(format!("cannot read {}", f.path)))?);
    }
    if !within_budget(&pixels) {
        return Err(AppError::Other(format!("{} MP in total, over the budget", pixels.iter().sum::<u64>() / 1_000_000)));
    }
    // The diff could have been generated while we weighed it.
    if up_to_date(app, review_id)? {
        return Ok(());
    }
    start(app, review_id, true);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_diff::store::{FileEntry, Change};

    fn req(id: i64) -> Request {
        Request { review_id: id, auto: false }
    }

    #[test]
    fn registry_runs_one_at_a_time_and_dedups() {
        let mut r = Registry::default();
        assert_eq!(r.enqueue(req(1)), Enqueued::StartWorker);
        assert_eq!(r.enqueue(req(2)), Enqueued::Queued);
        assert_eq!(r.enqueue(req(1)), Enqueued::Duplicate, "queued");
        let (first, _) = r.next().unwrap();
        assert_eq!(first.review_id, 1);
        assert_eq!(r.enqueue(req(1)), Enqueued::Duplicate, "running");
        assert!(r.is_active(2));
        assert_eq!(r.next().unwrap().0.review_id, 2);
        assert!(r.next().is_none());
        assert_eq!(r.enqueue(req(3)), Enqueued::StartWorker, "a retired worker is restarted");
    }

    #[test]
    fn cancel_flags_the_running_job_and_drops_queued_ones() {
        let mut r = Registry::default();
        r.enqueue(req(1));
        r.enqueue(req(2));
        let (_, flag) = r.next().unwrap();
        assert_eq!(r.cancel(2), Cancelled::Queued);
        assert_eq!(r.cancel(1), Cancelled::Running);
        assert!(flag.load(Ordering::Relaxed));
        assert_eq!(r.cancel(9), Cancelled::NotActive);
        assert!(r.next().is_none(), "the cancelled queued job never runs");
    }

    #[test]
    fn auto_budget_is_20_files_and_25_megapixels() {
        assert!(within_budget(&[]));
        assert!(within_budget(&[1_000_000; 20]));
        assert!(!within_budget(&[1_000; 21]));
        assert!(within_budget(&[25_000_000]));
        assert!(!within_budget(&[20_000_000, 5_000_001]));
        // Review 405: seven tall screenshots, about 200 MP.
        assert!(!within_budget(&[1222 * 36386, 1191 * 11774, 1191 * 13456, 1191 * 15138]));
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cr-status-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn manifest(pin: &str, skipped: bool) -> Manifest {
        let entry = |i: usize, skipped: bool| FileEntry {
            index: i,
            path: format!("{i}.png"),
            old_path: None,
            change: Change::Modified,
            before: None,
            after: None,
            mismatch_px: 0,
            aa_px: 0,
            diff_pct: 0.0,
            regions: vec![],
            other_regions: 0,
            skipped: skipped.then(|| "bad".to_string()),
        };
        Manifest {
            version: MANIFEST_VERSION,
            review_id: 1,
            pin: pin.into(),
            base_sha: Some("b".into()),
            head_sha: Some("h".into()),
            generated_at: "t".into(),
            preview_width: PREVIEW_WIDTH,
            auto: true,
            files: vec![entry(0, false), entry(1, skipped)],
        }
    }

    #[test]
    fn only_a_compared_pair_with_changed_pixels_is_worth_a_report() {
        let mut files = manifest("p", false).files;
        assert!(!has_changes_to_describe(&files), "nothing differs");
        files[1].mismatch_px = 3;
        assert!(has_changes_to_describe(&files));
        files[1].skipped = Some("bad".into());
        assert!(!has_changes_to_describe(&files), "a skipped file has no crops");
    }

    #[test]
    fn status_follows_the_files_and_the_pin() {
        let dir = tmp("flow");
        assert_eq!(status_of(&dir, "p", false).state, "none");

        store::write_progress(&dir, &Progress { state: "running".into(), done: 1, total: 4, error: None });
        let running = status_of(&dir, "p", true);
        assert_eq!((running.state, running.done, running.total), ("running", 1, 4));
        assert_eq!(status_of(&dir, "p", false).state, "interrupted", "a running file with no live job");

        store::write_progress(&dir, &Progress { state: "failed".into(), done: 0, total: 0, error: Some("boom".into()) });
        let failed = status_of(&dir, "p", false);
        assert_eq!((failed.state, failed.error.as_deref()), ("failed", Some("boom")));

        store::write_json(&dir, store::MANIFEST, &manifest("p", true)).unwrap();
        let ready = status_of(&dir, "p", false);
        assert_eq!((ready.state, ready.total, ready.done, ready.auto), ("ready", 2, 1, true));
        assert!(!ready.has_report);
        std::fs::write(dir.join(store::REPORT_FINAL), "{}").unwrap();
        assert!(status_of(&dir, "p", false).has_report);
        assert_eq!(status_of(&dir, "moved", false).state, "stale");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
