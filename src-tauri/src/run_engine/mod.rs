//! The run engine: FIFO queue under a concurrency cap, per-run directory, process-group control,
//! stop / timeout, tailing of the stream into numbered events, final status, pruning, and either
//! killing everything on shutdown or detaching so the next start re-adopts the runs.
//!
//! It has no Tauri dependency (a test enforces it): the host plugs in two seams.
//! - [`EventSink`]: what happens when a run is queued, started, emits an event or finishes, plus the
//!   host-specific work around a run (`prepare` before the process starts, `finished` after).
//! - [`RunStore`]: where run rows, events, settings and the run directory live.
//!
//! Processes are launched only through the [`Harness`](crate::ai::Harness) trait. Runs write to
//! files, not pipes, so they can outlive the process that started them.
#![allow(dead_code)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::ai::{Access, OneShot, SystemPrompt, SystemPromptMode, ToolPolicy};
use crate::ai::SessionMode;
use crate::ai_usage::{RunTag, RunUsage};
use crate::error::AppResult;

pub mod engine;
pub mod outcome;
pub mod proc;
pub mod tail;

/// Everything the engine needs to start a run's process, owned. Built by [`EventSink::prepare`].
pub struct Prepared {
    pub model: String,
    /// Written to the run directory and handed to the harness as a file.
    pub system_prompt: String,
    pub system_prompt_mode: SystemPromptMode,
    pub prompt: String,
    pub cwd: PathBuf,
    pub extra_dirs: Vec<PathBuf>,
    pub tools: ToolPolicy,
    pub access: Access,
    pub session_id: String,
    pub env: Vec<(String, OsString)>,
    pub extra_args: Vec<String>,
    pub usage_tag: RunTag,
    pub timeout: Duration,
    /// Scratch directories removed once the run is over.
    pub cleanup: Vec<PathBuf>,
}

impl Prepared {
    /// The harness request of this run; `prompt_path` is the system prompt file in the run directory.
    pub fn request<'a>(&'a self, prompt_path: &'a Path) -> OneShot<'a> {
        OneShot {
            model: &self.model,
            system_prompt: Some(SystemPrompt::File(prompt_path)),
            system_prompt_mode: self.system_prompt_mode,
            file_tag: "",
            prompt: &self.prompt,
            cwd: &self.cwd,
            extra_dirs: self.extra_dirs.iter().map(PathBuf::as_path).collect(),
            tools: self.tools.clone(),
            access: self.access.clone(),
            session: SessionMode::New(&self.session_id),
            max_turns: None,
            effort: None,
            env: self.env.clone(),
            extra_args: self.extra_args.clone(),
            usage_tag: self.usage_tag.clone(),
        }
    }
}

/// A run the store still lists as running (candidates for re-adoption).
#[derive(Debug, Clone)]
pub struct RunningRun {
    pub id: i64,
    pub pid: i32,
    pub session_id: Option<String>,
    pub started_at: Option<String>,
}

/// How a run ended, as recorded in the store and reported to the sink.
#[derive(Debug, Clone)]
pub struct Finished {
    pub status: &'static str,
    pub text: String,
    pub error: Option<String>,
    pub usage: RunUsage,
    /// What the usage log records as the failure, when the run did not succeed.
    pub usage_error: Option<String>,
    /// The host is exiting: no handlers, events or notifications.
    pub shutting_down: bool,
    /// Wall-clock stamp of when the engine started working on the run.
    pub started_at: String,
    pub elapsed: Duration,
}

/// Where run rows, events and the run directory live.
pub trait RunStore: Send + Sync {
    /// Whether queued runs may start (the host's AI switch).
    fn ai_enabled(&self) -> bool;
    fn max_concurrency(&self) -> i64;
    fn running_count(&self) -> AppResult<i64>;
    /// Oldest queued run.
    fn next_queued(&self) -> AppResult<Option<i64>>;
    /// `queued` -> `running`; false when the run is no longer queued.
    fn mark_running(&self, id: i64) -> AppResult<bool>;
    fn is_running(&self, id: i64) -> bool;
    /// `queued` -> `killed`; false when the run was not queued.
    fn drop_queued(&self, id: i64) -> AppResult<bool>;
    /// Queued plus running runs.
    fn active_count(&self) -> AppResult<usize>;
    /// Queued runs become `killed`.
    fn kill_queued(&self) -> AppResult<()>;
    /// Queued and running runs become `killed`.
    fn kill_active(&self) -> AppResult<()>;
    fn set_process(&self, id: i64, pid: u32, cwd: &Path, session_id: &str);
    fn set_cleanup(&self, id: i64, dirs: &[PathBuf]);
    fn cleanup_dirs(&self, id: i64) -> Vec<PathBuf>;
    /// Idempotent on `(run_id, seq)`; the store stamps the time.
    fn append_event(&self, id: i64, seq: i64, kind: &str, json: &str);
    fn last_seq(&self, id: i64) -> i64;
    fn running_runs(&self) -> AppResult<Vec<RunningRun>>;
    /// Write the final state of a `running` run (a row already settled is left alone).
    fn finish(&self, id: i64, fin: &Finished);
    /// Drop finished runs beyond the newest `keep`.
    fn prune(&self, keep: i64);
    /// A `running` run that can't be picked up again becomes `interrupted`.
    fn mark_lost(&self, id: i64);
    /// One subdirectory per live run lives under this root.
    fn runs_root(&self) -> PathBuf;
}

/// What happens around and during a run.
pub trait EventSink: Send + Sync {
    fn queued(&self, id: i64);
    fn started(&self, id: i64);
    fn status_changed(&self, id: i64, status: &str);
    fn event(&self, id: i64, seq: i64, kind: &str, json: &str);
    /// A queued run was stopped before it started (after `status_changed("killed")`).
    fn dropped(&self, id: i64);
    /// Turn a run that just started into a process request. `stopped` says whether a stop was
    /// requested meanwhile. `Ok(None)`: stopped before launch. `Err`: the run fails with this reason.
    fn prepare(&self, id: i64, stopped: &dyn Fn() -> bool) -> Result<Option<Prepared>, String>;
    /// A run left alive by an earlier session is being adopted: its timeout, or `None` when it can't
    /// be taken over.
    fn adopt(&self, id: i64) -> Option<Duration>;
    /// The run is over and recorded in the store.
    fn finished(&self, id: i64, fin: &Finished);
}
