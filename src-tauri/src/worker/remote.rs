//! `cr` inside a remote run (ADR 0007).
//!
//! The daemon sets three variables in a run's environment: [`ENV_RUN`] (the run id), [`ENV_DB`]
//! (the worker DB) and [`ENV_DIR`] (the run directory). When `cr` sees [`ENV_RUN`] it does not touch
//! a codereview DB: writes append structured ops to the run's outbox, reads are answered from the
//! context snapshot (`context.json` in the run directory) that came with the run request.
//!
//! Ops are idempotent by op id (`<run>:<n>`); the client applies them when it mirrors the run and
//! acknowledges them (`POST /v1/runs/{id}/outbox/ack`).
//!
//! Op types and args (all JSON objects):
//! - `board_meta_add`: `reference`, `kind`, `value`, `tag?`, `label?`, `cwd?`, `track`
//! - `board_meta_rm`: `id`
//! - `board_link`: `issue_id`, `pr_reference`
//! - `board_track`: `reference`
//! - `focus_set`: `review_id`, `plan` (the plan JSON), `level?`
//! - `focus_clear`: `review_id`
//! - `notify`: `message`, `title?`
//!
//! Snapshot shape: `{"cards": [<board show JSON>..], "focus": {"<review id>": <focus view JSON>}}`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::store;
use crate::error::{AppError, AppResult};

pub const ENV_RUN: &str = "CODEREVIEW_REMOTE_RUN";
pub const ENV_DB: &str = "CODEREVIEW_REMOTE_DB";
pub const ENV_DIR: &str = "CODEREVIEW_REMOTE_DIR";
pub const SNAPSHOT_FILE: &str = "context.json";

/// The environment the daemon gives a run's process.
pub fn run_env(run_id: i64, worker_db: &Path, run_dir: &Path) -> Vec<(String, OsString)> {
    vec![
        (ENV_RUN.into(), run_id.to_string().into()),
        (ENV_DB.into(), worker_db.into()),
        (ENV_DIR.into(), run_dir.into()),
    ]
}

/// Write the context snapshot into the run directory (created if needed).
pub fn write_snapshot(run_dir: &Path, context: &Value) -> std::io::Result<()> {
    std::fs::create_dir_all(run_dir)?;
    let snapshot = if context.is_null() { json!({}) } else { context.clone() };
    std::fs::write(run_dir.join(SNAPSHOT_FILE), snapshot.to_string())
}

/// The remote run `cr` is executing in.
#[derive(Debug, Clone)]
pub struct RemoteRun {
    pub run_id: i64,
    pub db: PathBuf,
    pub dir: PathBuf,
}

impl RemoteRun {
    /// `None` when `cr` is not inside a remote run.
    pub fn from_env() -> Option<Self> {
        Self::from_vars(|k| std::env::var_os(k))
    }

    pub fn from_vars(get: impl Fn(&str) -> Option<OsString>) -> Option<Self> {
        let run_id = get(ENV_RUN)?.to_str()?.parse().ok()?;
        Some(Self {
            run_id,
            db: get(ENV_DB).map(PathBuf::from)?,
            dir: get(ENV_DIR).map(PathBuf::from)?,
        })
    }

    /// Append an op to the run's outbox; returns its op id.
    pub fn append(&self, op_type: &str, args: Value) -> AppResult<String> {
        let conn = store::open(&self.db)?;
        store::append_outbox(&conn, self.run_id, op_type, &args)
    }

    pub fn snapshot(&self) -> AppResult<Value> {
        let path = self.dir.join(SNAPSHOT_FILE);
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| AppError::Other(format!("no context snapshot for this remote run ({}): {e}", path.display())))?;
        Ok(serde_json::from_str(&raw)?)
    }

    /// The snapshot's card for `owner/name#number`.
    pub fn card(&self, owner: &str, name: &str, number: i64) -> AppResult<Value> {
        let repo = format!("{owner}/{name}");
        self.snapshot()?
            .get("cards")
            .and_then(Value::as_array)
            .and_then(|cards| {
                cards.iter().find(|c| {
                    // `cr board show --json` is flat (the card's fields next to `lane` and `meta`); a nested
                    // `{"card": {..}}` is accepted too.
                    let card = if c.get("card").is_some_and(Value::is_object) { &c["card"] } else { c };
                    card["repo"].as_str().is_some_and(|r| r.eq_ignore_ascii_case(&repo)) && card["number"].as_i64() == Some(number)
                })
            })
            .cloned()
            .ok_or_else(|| AppError::Other(format!("{repo}#{number} is not in this remote run's context snapshot")))
    }

    pub fn cards(&self) -> AppResult<Vec<Value>> {
        Ok(self.snapshot()?.get("cards").and_then(Value::as_array).cloned().unwrap_or_default())
    }

    pub fn focus(&self, review_id: i64) -> AppResult<Value> {
        Ok(self.snapshot()?.get("focus").and_then(|f| f.get(review_id.to_string())).cloned().unwrap_or(Value::Null))
    }
}

/// The message for a `cr` command outside the supported subset.
pub fn unsupported(command: &str) -> AppError {
    AppError::Other(format!("`cr {command}` is not available in a remote run"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(dir: &Path, run_id: i64, db: &Path) -> RemoteRun {
        RemoteRun { run_id, db: db.to_path_buf(), dir: dir.to_path_buf() }
    }

    #[test]
    fn env_round_trips() {
        let env = run_env(7, Path::new("/d/worker.db"), Path::new("/d/runs/7"));
        let r = RemoteRun::from_vars(|k| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())).unwrap();
        assert_eq!((r.run_id, r.db, r.dir), (7, "/d/worker.db".into(), "/d/runs/7".into()));
        assert!(RemoteRun::from_vars(|_| None).is_none());
        assert!(RemoteRun::from_vars(|k| (k == ENV_RUN).then(|| "x".into())).is_none());
    }

    #[test]
    fn writes_are_recorded_in_the_outbox_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("worker.db");
        let sdb = store::WorkerDb::open(&db).unwrap();
        let (id, _) = sdb.insert_run("k", "claude", "", "{}", dir.path(), 60).unwrap();
        let r = remote(dir.path(), id, &db);
        let a = r.append("board_link", json!({"issue_id": "i", "pr_reference": "a/b#1"})).unwrap();
        let b = r.append("notify", json!({"message": "done"})).unwrap();
        assert_eq!((a.as_str(), b.as_str()), (format!("{id}:1").as_str(), format!("{id}:2").as_str()));
        let ops = sdb.outbox_after(id, 0).unwrap();
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0].op_type, "board_link");
        assert_eq!(ops[0].args["pr_reference"], "a/b#1");
        assert_eq!(ops[1].args["message"], "done");
    }

    #[test]
    fn reads_come_from_the_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = json!({
            "cards": [{"card": {"repo": "acme/widgets", "number": 3, "title": "T"}, "lane": "in_progress", "meta": [{"id": 1, "kind": "note", "value": "n"}]}],
            "focus": {"12": {"focus": {"status": "ready"}}}
        });
        write_snapshot(dir.path(), &ctx).unwrap();
        let r = remote(dir.path(), 1, Path::new("unused"));
        assert_eq!(r.card("ACME", "widgets", 3).unwrap()["card"]["title"], "T");
        assert!(r.card("acme", "widgets", 4).unwrap_err().to_string().contains("not in this remote run's context snapshot"));
        assert_eq!(r.cards().unwrap().len(), 1);
        assert_eq!(r.focus(12).unwrap()["focus"]["status"], "ready");
        assert!(r.focus(13).unwrap().is_null());
    }

    #[test]
    fn the_real_flat_board_json_is_found_too() {
        // The shape `BoardCardWithExtras` serializes to: `#[serde(flatten)]` puts the card's fields at the top.
        let flat = json!({"id": "c1", "type": "pr", "repo": "Acme/Widgets", "number": 3, "title": "T", "lane": "in_progress", "meta": []});
        let dir = tempfile::tempdir().unwrap();
        write_snapshot(dir.path(), &json!({"cards": [flat]})).unwrap();
        let r = remote(dir.path(), 1, Path::new("unused"));
        assert_eq!(r.card("acme", "widgets", 3).unwrap()["title"], "T");
        assert!(r.card("acme", "widgets", 4).is_err());
    }

    #[test]
    fn an_empty_context_gives_an_empty_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        write_snapshot(dir.path(), &Value::Null).unwrap();
        let r = remote(dir.path(), 1, Path::new("unused"));
        assert!(r.cards().unwrap().is_empty());
        assert!(unsupported("diff").to_string().contains("not available in a remote run"));
    }
}
