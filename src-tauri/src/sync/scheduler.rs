//! App-side scheduling (ADR 0003): decides when the pipeline runs. Views send
//! hints (`hint`), the window reports visibility and focus (`window`), and a
//! timer thread drives periodic light syncs and discovery (`tick`). The CLI
//! never uses this; it calls `pipeline::run` directly.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use super::{
    pipeline, MirrorUpdated, Mode, Scope, SyncReport, SyncRequest, BOARD_MONITOR_EVENT, MIRROR_UPDATED_EVENT,
    SYNC_STATUS_EVENT,
};
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
/// While notifications are polled, the discovery timer is only a safety net:
/// they record events, not current state (a withdrawn review request, a
/// changed watch setting), so the searches still run now and then.
const DISCOVERY_FALLBACK: Duration = Duration::from_secs(30 * 60);
const NOTIF_POLL_DEFAULT: Duration = Duration::from_secs(60);
const NOTIF_META_KEY: &str = "notifications_last_modified";

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
    /// Run the board auto-merge monitor, whatever `auto_sync` says.
    #[serde(default)]
    pub board_monitor: bool,
    #[serde(default = "default_monitor_interval_min")]
    pub board_monitor_interval_min: u32,
}

fn default_monitor_interval_min() -> u32 {
    5
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            auto_sync: AutoSync::Periodic,
            discovery_interval_min: 5,
            board_monitor: false,
            board_monitor_interval_min: default_monitor_interval_min(),
        }
    }
}

#[derive(Clone, Serialize)]
struct BoardMonitorPayload {
    actions: Vec<crate::board_monitor::MonitorAction>,
    error: Option<String>,
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
    last_monitor: Option<Instant>,
    monitoring: bool,
    /// Notifications poll (triggers discovery); `healthy` turns false for the
    /// session when a poll fails, which restores the plain discovery timer.
    last_notif_poll: Option<Instant>,
    notif_interval: Duration,
    notif_polling: bool,
    notif_healthy: bool,
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
        self.discovery_interval().is_some_and(|i| {
            let i = if self.notif_healthy { i.max(DISCOVERY_FALLBACK) } else { i };
            self.last_discovery.is_none_or(|t| t.elapsed() >= i)
        })
    }

    fn notif_poll_due(&self) -> bool {
        self.notif_healthy
            && !self.notif_polling
            && self.visible
            && self.config.auto_sync != AutoSync::Off
            && self.discovery_interval().is_some()
            && self.last_notif_poll.is_none_or(|t| t.elapsed() >= self.notif_interval)
    }

    fn monitor_due(&self) -> bool {
        self.config.board_monitor
            && !self.monitoring
            && self.last_monitor.is_none_or(|t| {
                t.elapsed() >= Duration::from_secs(u64::from(self.config.board_monitor_interval_min.max(1)) * 60)
            })
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
                    last_monitor: None,
                    monitoring: false,
                    last_notif_poll: None,
                    notif_interval: NOTIF_POLL_DEFAULT,
                    notif_polling: false,
                    notif_healthy: true,
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
        log::debug!(target: "sync", "configure {config:?}");
        let mut s = self.state();
        if config.board_monitor && !s.config.board_monitor {
            s.last_monitor = None;
        }
        s.config = config;
    }

    /// A view showing `scope` mounted (or changed what it shows), or went away
    /// (`active = false`). A mount syncs the scope if it is older than the TTL.
    pub fn hint(&self, scope: Scope, active: bool, visible_ids: Vec<String>) {
        let due = {
            let mut s = self.state();
            if !active {
                log::debug!(target: "sync", "hint {scope:?}: view gone");
                s.active.remove(&scope);
                return;
            }
            let n_ids = visible_ids.len();
            let mounted = s.active.insert(scope, visible_ids).is_none();
            let due = mounted && s.config.auto_sync != AutoSync::Off && s.due(scope, LIGHT_TTL);
            log::debug!(
                target: "sync",
                "hint {scope:?}: {n_ids} visible ids, newly mounted={mounted}, auto_sync={:?}, sync due={due}",
                s.config.auto_sync
            );
            due
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
            log::debug!(
                target: "sync",
                "window visible={visible} focused={focused} (gained focus={gained}, away {:?})",
                away
            );
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
            let discover = !s.notif_healthy
                && s.discovery_interval().is_some_and(|i| away.is_some_and(|a| a >= i));
            let scopes: Vec<Scope> = s.active.keys().copied().filter(|sc| s.due(*sc, LIGHT_TTL)).collect();
            (discover, scopes)
        };
        self.spawn_round(discover, scopes);
        self.maybe_poll_notifications();
    }

    fn tick(&self) {
        self.maybe_monitor();
        self.maybe_poll_notifications();
        let (discover, scopes) = {
            let s = self.state();
            if !s.visible || s.config.auto_sync != AutoSync::Periodic {
                return;
            }
            let scopes: Vec<Scope> =
                s.active.keys().copied().filter(|sc| s.due(*sc, s.periodic_interval(*sc))).collect();
            (s.discovery_due(), scopes)
        };
        if discover || !scopes.is_empty() {
            log::debug!(target: "sync", "tick: discover={discover}, due scopes={scopes:?}");
        }
        self.spawn_round(discover, scopes);
    }

    /// Poll `/notifications` if due; a change since the last poll starts a
    /// discovery. The very first poll (no stored `Last-Modified`) only records
    /// the baseline: the fallback timer covers startup.
    fn maybe_poll_notifications(&self) {
        {
            let mut s = self.state();
            if !s.notif_poll_due() {
                return;
            }
            s.notif_polling = true;
            s.last_notif_poll = Some(Instant::now());
        }
        let engine = self.clone();
        let spawned = std::thread::Builder::new().name("notifications-poll".into()).spawn(move || {
            engine.poll_notifications_round();
            engine.state().notif_polling = false;
        });
        if let Err(e) = spawned {
            self.state().notif_polling = false;
            crate::logging::warn(&format!("[sync] could not start the notifications poll: {e}"));
        }
    }

    fn poll_notifications_round(&self) {
        let db = self.inner.app.state::<Db>();
        let since = crate::commands::inbox::get_meta(&db.0.lock().unwrap(), NOTIF_META_KEY);
        match crate::gh::poll_notifications(since.as_deref()) {
            Ok(crate::gh::NotificationsPoll::NotModified) => {
                log::debug!(target: "sync", "notifications: 304");
            }
            Ok(crate::gh::NotificationsPoll::Changed { last_modified, poll_interval }) => {
                log::debug!(target: "sync", "notifications: changed (baseline={})", since.is_none());
                if let Some(lm) = &last_modified {
                    let _ = crate::commands::inbox::set_meta(&db.0.lock().unwrap(), NOTIF_META_KEY, lm);
                }
                if let Some(secs) = poll_interval {
                    self.state().notif_interval = Duration::from_secs(secs).max(NOTIF_POLL_DEFAULT);
                }
                if since.is_some() {
                    self.spawn(SyncRequest { discover: true, ..SyncRequest::new(Scope::All, Mode::Light) });
                }
            }
            Err(e) => {
                crate::logging::warn(&format!(
                    "[sync] notifications poll failed, falling back to the discovery timer: {e}"
                ));
                self.state().notif_healthy = false;
            }
        }
    }

    /// Start a monitor round if it is on and due. Unlike light syncs this
    /// ignores window visibility, the active views and Auto Sync: turning the
    /// monitor on is the opt-in.
    fn maybe_monitor(&self) {
        {
            let mut s = self.state();
            if !s.monitor_due() {
                return;
            }
            s.monitoring = true;
            s.last_monitor = Some(Instant::now());
        }
        let engine = self.clone();
        let spawned = std::thread::Builder::new().name("board-monitor".into()).spawn(move || {
            engine.monitor_round();
            engine.state().monitoring = false;
        });
        if let Err(e) = spawned {
            self.state().monitoring = false;
            crate::logging::warn(&format!("[sync] could not start the board monitor: {e}"));
        }
    }

    /// Refresh the board mirror, then act on what it shows.
    fn monitor_round(&self) {
        let req = SyncRequest::new(Scope::Board, Mode::Light);
        if !self.begin(&req, true) {
            log::debug!(target: "sync", "monitor: skipped, a board sync is running");
            self.state().last_monitor = None;
            return;
        }
        let mut error = self.execute(req).err().map(|e| e.to_string());
        if error.is_none() {
            let db = self.inner.app.state::<Db>();
            match crate::board_monitor::run(&db) {
                Ok(actions) => {
                    log::info!("[sync] monitor: {} actions", actions.len());
                    if !actions.is_empty() {
                        let mut ids: Vec<String> = actions.iter().map(|a| a.card_id.clone()).collect();
                        ids.dedup();
                        let _ = self
                            .inner
                            .app
                            .emit(MIRROR_UPDATED_EVENT, &MirrorUpdated { scopes: vec![Scope::Board], ids });
                        let _ = self.inner.app.emit(BOARD_MONITOR_EVENT, BoardMonitorPayload { actions, error: None });
                    }
                    return;
                }
                Err(e) => {
                    crate::logging::warn(&format!("[sync] monitor failed: {e}"));
                    self.state().last_error = Some((e.to_string(), chrono::Utc::now().to_rfc3339()));
                    self.emit_status();
                    error = Some(e.to_string());
                }
            }
        }
        let _ = self
            .inner
            .app
            .emit(BOARD_MONITOR_EVENT, BoardMonitorPayload { actions: Vec::new(), error });
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
            log::debug!(target: "sync", "spawn {:?} {:?}: skipped, overlapping sync running", req.scope, req.mode);
            return;
        }
        log::debug!(
            target: "sync",
            "spawn {:?} {:?} discover={} ({} visible ids)",
            req.scope, req.mode, req.discover, req.visible_ids.len()
        );
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
        log::debug!(
            target: "sync",
            "run_now {:?} {:?} discover={} ({} visible ids)",
            req.scope, req.mode, req.discover, req.visible_ids.len()
        );
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
            log::debug!(target: "sync", "emit {MIRROR_UPDATED_EVENT} {u:?}");
            let _ = app.emit(MIRROR_UPDATED_EVENT, &u);
        };
        let started = Instant::now();
        log::debug!(target: "sync", "execute {:?} {:?} start", req.scope, req.mode);
        let result = pipeline::run(&db, &req, &notify);
        log::debug!(target: "sync", "execute {:?} {:?} done in {}ms (ok={})", req.scope, req.mode, started.elapsed().as_millis(), result.is_ok());
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
                        "[sync] {:?} {:?}{}: {} probed, {} refetched, {} threads in {}ms",
                        req.scope,
                        req.mode,
                        if report.discovered { " + discovery" } else { "" },
                        report.probed,
                        report.changed,
                        report.threads,
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
        let status = self.status();
        log::debug!(target: "sync", "emit {SYNC_STATUS_EVENT} running={:?} discovering={}", status.running, status.discovering);
        let _ = self.inner.app.emit(SYNC_STATUS_EVENT, status);
    }
}
