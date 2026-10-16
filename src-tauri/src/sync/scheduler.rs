//! App-side scheduling (ADR 0003): decides when the pipeline runs. Views send
//! hints (`hint`), the window reports visibility and focus (`window`), and a
//! timer thread drives periodic light syncs and discovery (`tick`). The CLI
//! never uses this; it calls `pipeline::run` directly.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use super::{pipeline, Mode, Scope, SyncReport, SyncRequest, MIRROR_UPDATED_EVENT, SYNC_STATUS_EVENT};
use crate::db::Db;
use crate::error::AppResult;

/// A view mount or window focus syncs a scope only when its last sync is older.
const LIGHT_TTL: Duration = Duration::from_secs(60);
/// Periodic light sync interval per scope, doubled per unchanged sync up to
/// its max backoff, and again while the window is unfocused. The board's PR
/// states should look live, and its light sync is only a probe (~5 calls).
fn periodic_base(scope: Scope) -> (Duration, u32) {
    match scope {
        Scope::Board => (Duration::from_secs(60), 2),
        _ => (Duration::from_secs(3 * 60), 4),
    }
}
const TICK: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoSync {
    Off,
    OnFocus,
    #[default]
    Periodic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncConfig {
    pub auto_sync: AutoSync,
    /// Minutes between discoveries while the app is visible; 0 = off.
    pub discovery_interval_min: u32,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self { auto_sync: AutoSync::Periodic, discovery_interval_min: 15 }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    /// Scopes with a sync in flight.
    pub running: Vec<Scope>,
    pub discovering: bool,
    pub full: bool,
    pub last_discovery_at: Option<String>,
    pub last_inbox_at: Option<String>,
    pub last_board_at: Option<String>,
    pub last_error: Option<String>,
    pub last_error_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct Running {
    scope: Scope,
    mode: Mode,
    discover: bool,
}

struct State {
    config: SyncConfig,
    visible: bool,
    focused: bool,
    blurred_at: Option<Instant>,
    /// Scopes whose view is on screen, with the entity ids it shows.
    active: HashMap<Scope, Vec<String>>,
    last_sync: HashMap<Scope, Instant>,
    backoff: HashMap<Scope, u32>,
    last_discovery: Option<Instant>,
    running: Vec<Running>,
    last_error: Option<(String, String)>,
}

impl State {
    fn due(&self, scope: Scope, after: Duration) -> bool {
        self.last_sync.get(&scope).is_none_or(|t| t.elapsed() >= after)
    }

    fn periodic_interval(&self, scope: Scope) -> Duration {
        let backoff = self.backoff.get(&scope).copied().unwrap_or(1);
        periodic_base(scope).0 * backoff * if self.focused { 1 } else { 2 }
    }

    fn discovery_interval(&self) -> Option<Duration> {
        (self.config.discovery_interval_min > 0)
            .then(|| Duration::from_secs(u64::from(self.config.discovery_interval_min) * 60))
    }

    fn discovery_due(&self) -> bool {
        self.discovery_interval()
            .is_some_and(|i| self.last_discovery.is_none_or(|t| t.elapsed() >= i))
    }

    fn overlaps(&self, scope: Scope) -> bool {
        self.running.iter().any(|r| r.scope.covers(scope) || scope.covers(r.scope))
    }
}

struct Inner {
    app: AppHandle,
    state: Mutex<State>,
}

#[derive(Clone)]
pub struct SyncEngine {
    inner: Arc<Inner>,
}

/// How long ago an RFC 3339 timestamp was, as an `Instant`.
fn instant_of(ts: Option<String>) -> Option<Instant> {
    let at = chrono::DateTime::parse_from_rfc3339(&ts?).ok()?;
    let ago = (chrono::Utc::now() - at.with_timezone(&chrono::Utc)).to_std().ok()?;
    Instant::now().checked_sub(ago)
}

impl SyncEngine {
    /// Build the engine and start its timer thread. `Db` must be managed.
    pub fn start(app: AppHandle) -> Self {
        let last_discovery = {
            let db = app.state::<Db>();
            let conn = db.0.lock().unwrap();
            instant_of(crate::commands::inbox::get_meta(&conn, "last_discovery_at"))
        };
        let engine = Self {
            inner: Arc::new(Inner {
                app,
                state: Mutex::new(State {
                    config: SyncConfig::default(),
                    visible: true,
                    focused: false,
                    blurred_at: None,
                    active: HashMap::new(),
                    last_sync: HashMap::new(),
                    backoff: HashMap::new(),
                    last_discovery,
                    running: Vec::new(),
                    last_error: None,
                }),
            }),
        };
        let ticker = engine.clone();
        let spawned = std::thread::Builder::new().name("sync-scheduler".into()).spawn(move || loop {
            std::thread::sleep(TICK);
            ticker.tick();
        });
        if let Err(e) = spawned {
            crate::logging::warn(&format!("[sync] could not start the scheduler: {e}"));
        }
        engine
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner.state.lock().unwrap()
    }

    pub fn configure(&self, config: SyncConfig) {
        self.state().config = config;
    }

    /// A view showing `scope` mounted (or changed what it shows), or went away
    /// (`active = false`). A mount syncs the scope if it is older than the TTL.
    pub fn hint(&self, scope: Scope, active: bool, visible_ids: Vec<String>) {
        let due = {
            let mut s = self.state();
            if !active {
                s.active.remove(&scope);
                return;
            }
            let mounted = s.active.insert(scope, visible_ids).is_none();
            mounted && s.config.auto_sync != AutoSync::Off && s.due(scope, LIGHT_TTL)
        };
        if due {
            self.spawn(SyncRequest::new(scope, Mode::Light));
        }
    }

    /// Window visibility/focus changed. Regaining focus syncs the stale active
    /// scopes, or runs discovery if the app was away longer than its interval.
    pub fn window(&self, visible: bool, focused: bool) {
        let (discover, scopes) = {
            let mut s = self.state();
            let gained = focused && !s.focused;
            let away = s.blurred_at.map(|t| t.elapsed());
            if focused {
                s.blurred_at = None;
            } else if s.focused || s.blurred_at.is_none() {
                s.blurred_at = Some(Instant::now());
            }
            s.visible = visible;
            s.focused = focused;
            if !gained || s.config.auto_sync == AutoSync::Off {
                return;
            }
            let discover = s.discovery_interval().is_some_and(|i| away.is_some_and(|a| a >= i));
            let scopes: Vec<Scope> = s.active.keys().copied().filter(|sc| s.due(*sc, LIGHT_TTL)).collect();
            (discover, scopes)
        };
        self.spawn_round(discover, scopes);
    }

    fn tick(&self) {
        let (discover, scopes) = {
            let s = self.state();
            if !s.visible || s.config.auto_sync != AutoSync::Periodic {
                return;
            }
            let scopes: Vec<Scope> =
                s.active.keys().copied().filter(|sc| s.due(*sc, s.periodic_interval(*sc))).collect();
            (s.discovery_due(), scopes)
        };
        self.spawn_round(discover, scopes);
    }

    /// Discovery covers every scope (it syncs Inbox + Board), so it replaces
    /// the per-scope light syncs of this round.
    fn spawn_round(&self, discover: bool, scopes: Vec<Scope>) {
        if discover {
            self.spawn(SyncRequest { discover: true, ..SyncRequest::new(Scope::All, Mode::Light) });
        } else {
            for scope in scopes {
                self.spawn(SyncRequest::new(scope, Mode::Light));
            }
        }
    }

    /// Start a background sync unless one overlapping its scope is running.
    pub fn spawn(&self, req: SyncRequest) {
        if !self.begin(&req, true) {
            return;
        }
        let engine = self.clone();
        let spawned = std::thread::Builder::new().name("sync".into()).spawn(move || {
            let _ = engine.execute(req);
        });
        if let Err(e) = spawned {
            crate::logging::warn(&format!("[sync] could not start a sync: {e}"));
        }
    }

    /// Run a sync now and wait for it (a Sync click). Overlapping work with a
    /// sync already in flight is shared, not repeated.
    pub fn run_now(&self, mut req: SyncRequest) -> AppResult<SyncReport> {
        if req.mode == Mode::Forced && req.visible_ids.is_empty() {
            if let Some(ids) = self.state().active.get(&req.scope) {
                req.visible_ids = ids.clone();
            }
        }
        self.begin(&req, false);
        self.execute(req)
    }

    fn begin(&self, req: &SyncRequest, exclusive: bool) -> bool {
        {
            let mut s = self.state();
            if exclusive && s.overlaps(req.scope) {
                return false;
            }
            s.running.push(Running { scope: req.scope, mode: req.mode, discover: req.discover });
        }
        self.emit_status();
        true
    }

    fn execute(&self, req: SyncRequest) -> AppResult<SyncReport> {
        let app = self.inner.app.clone();
        let db = app.state::<Db>();
        let notify = |u| {
            let _ = app.emit(MIRROR_UPDATED_EVENT, &u);
        };
        let result = pipeline::run(&db, &req, &notify);
        {
            let mut s = self.state();
            let me = Running { scope: req.scope, mode: req.mode, discover: req.discover };
            if let Some(i) = s.running.iter().position(|r| *r == me) {
                s.running.remove(i);
            }
            let now = Instant::now();
            match &result {
                Ok(report) => {
                    log::info!(
                        "[sync] {:?} {:?}{}: {} probed, {} refetched, {} threads, {} files in {}ms",
                        req.scope,
                        req.mode,
                        if report.discovered { " + discovery" } else { "" },
                        report.probed,
                        report.changed,
                        report.threads,
                        report.files,
                        report.duration_ms,
                    );
                    s.last_error = None;
                    for scope in [Scope::Inbox, Scope::Board] {
                        if !req.scope.covers(scope) {
                            continue;
                        }
                        s.last_sync.insert(scope, now);
                        let b = s.backoff.entry(scope).or_insert(1);
                        *b = if req.mode == Mode::Light && report.changed == 0 { (*b * 2).min(periodic_base(scope).1) } else { 1 };
                    }
                    if report.discovered && req.scope.inbox() {
                        s.last_discovery = Some(now);
                    }
                }
                Err(e) => {
                    crate::logging::warn(&format!("[sync] {:?} {:?} sync failed: {e}", req.scope, req.mode));
                    s.last_error = Some((e.to_string(), chrono::Utc::now().to_rfc3339()));
                }
            }
        }
        self.emit_status();
        result
    }

    pub fn status(&self) -> SyncStatus {
        let (running, discovering, full, last_error) = {
            let s = self.state();
            let mut scopes: Vec<Scope> = Vec::new();
            for r in &s.running {
                if !scopes.contains(&r.scope) {
                    scopes.push(r.scope);
                }
            }
            (
                scopes,
                s.running.iter().any(|r| r.discover || r.mode != Mode::Light),
                s.running.iter().any(|r| r.mode == Mode::Full),
                s.last_error.clone(),
            )
        };
        let db = self.inner.app.state::<Db>();
        let conn = db.0.lock().unwrap();
        let meta = |k| crate::commands::inbox::get_meta(&conn, k);
        SyncStatus {
            running,
            discovering,
            full,
            last_discovery_at: meta("last_discovery_at"),
            last_inbox_at: meta("last_refresh_at"),
            last_board_at: meta("board_last_refresh_at"),
            last_error_at: last_error.as_ref().map(|e| e.1.clone()),
            last_error: last_error.map(|e| e.0),
        }
    }

    fn emit_status(&self) {
        let _ = self.inner.app.emit(SYNC_STATUS_EVENT, self.status());
    }
}
