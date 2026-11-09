//! The daemon's [`EventSink`]: turns a stored request into a process request, and tells the SSE
//! streams (through a change channel) when a run changed. Events themselves are read back from the
//! store, so a stream replays from any `seq` and then follows live.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub use changes::Changes;

use super::remote;
use super::request::RunRequest;
use super::store::ServeStore;
use crate::ai::{Harness, SystemPromptMode};
use crate::commands::chat::new_session_id;
use crate::commands::focus::child_path;
use crate::run_engine::{EventSink, Finished, Prepared, RunStore};
use crate::run_usage::RunTag;

/// A wake-up for the streams: the id of the run that changed. A plain `std` channel list keeps the
/// sink usable without the HTTP stack.
pub mod changes {
    use std::sync::{Arc, Mutex};

    type Listener = Box<dyn Fn(i64) + Send + Sync>;

    #[derive(Default, Clone)]
    pub struct Changes(Arc<Mutex<Vec<Listener>>>);

    impl Changes {
        pub fn subscribe(&self, f: impl Fn(i64) + Send + Sync + 'static) {
            self.0.lock().unwrap().push(Box::new(f));
        }

        pub fn notify(&self, id: i64) {
            for f in self.0.lock().unwrap().iter() {
                f(id);
            }
        }
    }
}

pub struct ServeSink {
    pub store: Arc<ServeStore>,
    pub changes: Changes,
    /// The serve DB file, handed to `cr` inside the run so it can append to the outbox.
    pub db_path: PathBuf,
    /// The harnesses a run may name (the registry plus whatever a test plugs in).
    pub harnesses: Vec<&'static dyn Harness>,
}

impl ServeSink {
    fn request(&self, id: i64) -> Result<RunRequest, String> {
        let raw = self.store.db.request_json(id).map_err(|e| e.to_string())?.ok_or("the run no longer exists")?;
        serde_json::from_str(&raw).map_err(|e| format!("unreadable stored request: {e}"))
    }
}

impl EventSink for ServeSink {
    fn queued(&self, id: i64) {
        self.changes.notify(id);
    }

    fn started(&self, id: i64) {
        self.changes.notify(id);
    }

    fn status_changed(&self, id: i64, _status: &str) {
        self.changes.notify(id);
    }

    fn event(&self, id: i64, _seq: i64, _kind: &str, _json: &str) {
        self.changes.notify(id);
    }

    fn dropped(&self, id: i64) {
        // A queued run never reached `prepare`, so nothing else would remove its attachments.
        let _ = std::fs::remove_dir_all(super::workspace::scratch_dir(&self.store.runs_root(), id));
        self.changes.notify(id);
    }

    fn prepare(&self, id: i64, stopped: &dyn Fn() -> bool) -> Result<Option<Prepared>, String> {
        let req = self.request(id)?;
        let cwd = self.store.db.cwd(id).map_err(|e| e.to_string())?.ok_or("the run no longer exists")?;
        if stopped() {
            return Ok(None);
        }
        let harness = self
            .harnesses
            .iter()
            .copied()
            .find(|h| h.id() == req.harness)
            .ok_or_else(|| format!("unknown harness `{}`", req.harness))?;
        let dir = self.store.runs_root().join(id.to_string());
        remote::write_snapshot(&dir, &req.context).map_err(|e| format!("failed to write the context snapshot: {e}"))?;
        let mut env: Vec<(String, std::ffi::OsString)> = remote::run_env(id, &self.db_path, &dir);
        if let Some(path) = child_path() {
            env.push(("PATH".into(), path));
        }
        let scratch = super::workspace::scratch_dir(&self.store.runs_root(), id);
        let has_scratch = scratch.is_dir();
        Ok(Some(Prepared {
            harness,
            model: req.model.clone(),
            system_prompt: req.system_prompt.clone(),
            system_prompt_mode: if req.system_prompt_replace { SystemPromptMode::Replace } else { SystemPromptMode::Append },
            prompt: req.prompt.clone(),
            cwd,
            extra_dirs: if has_scratch { vec![scratch.clone()] } else { Vec::new() },
            tools: req.tools.policy(&req.allowed_tools),
            access: req.access.access(),
            session_id: new_session_id(),
            env,
            extra_args: Vec::new(),
            usage_tag: RunTag::new("remote_run").harness(harness.id()).label(format!("run {id}")),
            timeout: Duration::from_secs(req.timeout_secs.max(1)),
            cleanup: if has_scratch { vec![scratch] } else { Vec::new() },
        }))
    }

    fn adopt(&self, id: i64) -> Option<Duration> {
        self.request(id).ok().map(|r| Duration::from_secs(r.timeout_secs.max(1)))
    }

    fn finished(&self, id: i64, _fin: &Finished) {
        self.changes.notify(id);
    }
}
