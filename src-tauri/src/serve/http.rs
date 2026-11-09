//! The `/v1` HTTP+SSE API (axum). Handlers are thin: validation, the store and the run engines do
//! the work. Blocking work (git, the app DB, a harness call) runs on the blocking pool.

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path as UrlPath, Query, Request, State};
use axum::middleware::Next;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use super::request::{OneShotRequest, OneShotResponse, OneShotUsage, RunRequest, ToolsWire};
use super::sink::{Changes, ServeSink};
use super::store::{is_terminal, ServeDb, ServeStore};
use super::{workspace, ApiError, ErrorCode, PROTOCOL};
use crate::ai::{check_caps, Harness, Needs, OneShot, SessionMode, SystemPrompt, SystemPromptMode};
use crate::error::{AppError, AppResult};
use crate::run_engine::engine::RunEngine;
use crate::run_usage::RunTag;

pub type PrHeadFn = Box<dyn Fn(&Path, i64) -> AppResult<(String, String)> + Send + Sync>;

/// How long `/v1/oneshot` waits for the harness before answering `timeout`.
pub const ONESHOT_TIMEOUT: Duration = Duration::from_secs(600);

/// Queued plus running runs beyond which new ones are refused with `queue_full`.
pub const MAX_QUEUE: i64 = 100;

pub struct Shared {
    pub db: Arc<ServeDb>,
    /// The interactive app DB, read for the repo registry.
    pub app_db: PathBuf,
    pub serve_db_path: PathBuf,
    pub harnesses: Vec<&'static dyn Harness>,
    /// The one run engine; each run names its own harness.
    pub engine: Arc<RunEngine>,
    pub changes: broadcast::Sender<i64>,
    pub cap: Arc<AtomicI64>,
    pub worktree_base: PathBuf,
    pub pr_head: PrHeadFn,
    pub max_queue: AtomicI64,
    /// Serialises workspace resolution: two requests for the same branch must not both run `git worktree add`.
    /// There is no lock on the worktree itself: a queued run does not hold its worktree (ADR 0007).
    /// Bounds concurrent one-shot calls (they run outside the run engine).
    pub oneshot_slots: Arc<tokio::sync::Semaphore>,
    pub oneshot_timeout: Duration,
    pub workspace_lock: std::sync::Mutex<()>,
    /// `<data_dir>/serve_runs`: the run directories (`<id>`) and the attachment scratch dirs (`scratch/<id>`).
    pub runs_base: PathBuf,
}

pub type SharedState = Arc<Shared>;

pub struct Settings {
    pub app_db: PathBuf,
    pub serve_db_path: PathBuf,
    /// Runs, scratch dirs and the like live under `<data_dir>/serve_runs`.
    pub data_dir: PathBuf,
    pub max_concurrency: i64,
    pub oneshot_timeout: Duration,
    pub worktree_base: PathBuf,
    pub pr_head: PrHeadFn,
}

/// Build the shared state: one run engine per available harness, each over its own view of the store.
pub fn build(settings: Settings, harnesses: Vec<&'static dyn Harness>) -> AppResult<SharedState> {
    let db = ServeDb::open(&settings.serve_db_path)?;
    let cap = Arc::new(AtomicI64::new(settings.max_concurrency.max(1)));
    let (tx, _) = broadcast::channel(256);
    let changes = Changes::default();
    {
        let tx = tx.clone();
        changes.subscribe(move |id| {
            let _ = tx.send(id);
        });
    }
    let default = *harnesses.first().ok_or_else(|| AppError::Other("no harness".into()))?;
    let store = ServeStore::new(db.clone(), cap.clone(), settings.data_dir.join("serve_runs"));
    let sink = Arc::new(ServeSink {
        store: store.clone(),
        changes: changes.clone(),
        db_path: settings.serve_db_path.clone(),
        harnesses: harnesses.clone(),
    });
    let engine = RunEngine::new(store, sink, default);
    Ok(Arc::new(Shared {
        db,
        app_db: settings.app_db,
        serve_db_path: settings.serve_db_path,
        harnesses,
        engine,
        changes: tx,
        cap,
        worktree_base: settings.worktree_base,
        pr_head: settings.pr_head,
        max_queue: AtomicI64::new(MAX_QUEUE),
        oneshot_slots: Arc::new(tokio::sync::Semaphore::new(settings.max_concurrency.max(1) as usize)),
        oneshot_timeout: settings.oneshot_timeout,
        workspace_lock: std::sync::Mutex::new(()),
        runs_base: settings.data_dir.join("serve_runs"),
    }))
}

impl Shared {
    fn harness(&self, id: &str) -> Result<&'static dyn Harness, ApiError> {
        let h = self
            .harnesses
            .iter()
            .copied()
            .find(|h| h.id() == id)
            .ok_or_else(|| ApiError::new(ErrorCode::HarnessUnavailable, format!("unknown harness `{id}`")))?;
        if !h.available() {
            return Err(ApiError::new(ErrorCode::HarnessUnavailable, format!("{} is not installed on this host", h.name())));
        }
        Ok(h)
    }

    /// `read_only`: the run can't write, so a branch checked out in the main clone may be shared.
    fn resolve_workspace(&self, spec: &super::request::WorkspaceSpec, read_only: bool) -> Result<PathBuf, ApiError> {
        let _guard = self.workspace_lock.lock().unwrap_or_else(|e| e.into_inner());
        let conn = crate::db::open(&self.app_db)?;
        workspace::resolve_in(&conn, spec, &self.worktree_base, &*self.pr_head, read_only)
    }

    fn scratch(&self, run_id: i64) -> PathBuf {
        workspace::scratch_dir(&self.runs_base, run_id)
    }
}

// ---------------------------------------------------------------------------------------------
// Errors and parsing
// ---------------------------------------------------------------------------------------------

pub struct HttpError(pub ApiError);

impl From<ApiError> for HttpError {
    fn from(e: ApiError) -> Self {
        Self(e)
    }
}

impl From<crate::error::AppError> for HttpError {
    fn from(e: crate::error::AppError) -> Self {
        Self(e.into())
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.0.code.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(json!({ "error": { "code": self.0.code.as_str(), "message": self.0.message } }))).into_response()
    }
}

type HttpResult<T> = Result<T, HttpError>;

fn parse<T: DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|e| ApiError::bad_request(format!("invalid request body: {e}")))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ApiError> + Send + 'static) -> HttpResult<T> {
    match tokio::task::spawn_blocking(f).await {
        Ok(r) => r.map_err(HttpError),
        Err(e) => Err(HttpError(ApiError::internal(e))),
    }
}

// ---------------------------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------------------------

/// Browser-facing defences for the loopback TCP listener (the unix socket is out of a browser's reach).
/// - Any `Origin` header is refused: browsers add it to cross-origin requests, the CLIs and the app never do.
/// - `Host` must name a loopback address, which defeats DNS rebinding (TCP only).
/// - A POST with a body must be `application/json`, so a plain HTML form cannot reach a handler.
async fn guard(State(tcp): State<bool>, req: Request, next: Next) -> Response {
    let refuse = |code, msg: &str| HttpError(ApiError::new(code, msg)).into_response();
    let headers = req.headers();
    if headers.contains_key("origin") {
        return refuse(ErrorCode::Forbidden, "cross-origin requests are refused");
    }
    if tcp {
        let host = headers.get("host").and_then(|v| v.to_str().ok()).unwrap_or("");
        if !host_is_loopback(host) {
            return refuse(ErrorCode::Forbidden, "the Host header must name a loopback address");
        }
    }
    if req.method() == axum::http::Method::POST {
        let has_body = headers.get("content-length").and_then(|v| v.to_str().ok()).map(|v| v.trim() != "0").unwrap_or(false)
            || headers.contains_key("transfer-encoding");
        let json = headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case("application/json"))
            .unwrap_or(false);
        if has_body && !json {
            return refuse(ErrorCode::BadRequest, "Content-Type must be application/json");
        }
    }
    next.run(req).await
}

/// `localhost`, `127.x.y.z` or `[::1]`, with or without a port.
fn host_is_loopback(host: &str) -> bool {
    let name = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(""),
        None => host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host),
    };
    name.eq_ignore_ascii_case("localhost") || name.parse::<std::net::IpAddr>().map(|ip| ip.is_loopback()).unwrap_or(false)
}

/// `tcp`: the listener is a loopback TCP port, so the `Host` check applies.
pub fn router(state: SharedState, tcp: bool) -> Router {
    Router::new()
        .route("/v1/info", get(info))
        .route("/v1/harnesses/{id}/models", get(models))
        .route("/v1/oneshot", post(oneshot))
        .route("/v1/runs", get(list_runs).post(create_run))
        .route("/v1/runs/{id}", get(get_run))
        .route("/v1/runs/{id}/events", get(run_events))
        .route("/v1/runs/{id}/stop", post(stop_run))
        .route("/v1/runs/{id}/outbox", get(outbox))
        .route("/v1/runs/{id}/outbox/ack", post(outbox_ack))
        .route("/v1/sessions", get(sessions))
        .fallback(|| async { HttpError(ApiError::new(ErrorCode::NotFound, "no such endpoint")) })
        .layer(DefaultBodyLimit::max(workspace::MAX_ATTACHMENTS_TOTAL * 2))
        .layer(axum::middleware::from_fn_with_state(tcp, guard))
        .with_state(state)
}

fn host_name() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".into())
}

async fn info(State(s): State<SharedState>) -> HttpResult<Json<Value>> {
    blocking(move || {
        let harnesses: Vec<Value> = s
            .harnesses
            .iter()
            .map(|h| {
                let available = h.available();
                json!({
                    "id": h.id(),
                    "name": h.name(),
                    "caps": h.caps(),
                    "available": available,
                    "version": if available { h.version() } else { None },
                })
            })
            .collect();
        let (running, queued) = s.db.counts();
        Ok(Json(json!({
            "protocol": PROTOCOL,
            "version": env!("CODEREVIEW_VERSION"),
            "host": host_name(),
            "harnesses": harnesses,
            "max_concurrency": s.cap.load(Ordering::Relaxed),
            "load": { "running": running, "queued": queued },
        })))
    })
    .await
}

/// Model ids of a harness: the registry's cached listing (`ai::models`); a harness outside the
/// registry (a test double) answers for itself.
fn models_of(h: &dyn Harness) -> AppResult<Vec<String>> {
    if crate::ai::harness_by_id(h.id()).is_some() {
        Ok(crate::ai::models::for_harness(h.id(), false)?.into_iter().flat_map(|g| g.models).collect())
    } else {
        h.models()
    }
}

async fn models(State(s): State<SharedState>, UrlPath(id): UrlPath<String>) -> HttpResult<Json<Value>> {
    blocking(move || {
        let h = s.harness(&id)?;
        Ok(Json(json!({ "harness": id, "models": models_of(h)? })))
    })
    .await
}

fn needs_of(access: crate::ai::Access, tools: ToolsWire) -> Needs {
    Needs { access, tools: tools != ToolsWire::None, tool_allowlist: tools == ToolsWire::Allowlist }
}

/// Whether a request can only read: the one case that may share the user's main checkout.
fn read_only_of(access: super::request::AccessWire, tools: ToolsWire) -> bool {
    access == super::request::AccessWire::ReadOnly && matches!(tools, ToolsWire::None | ToolsWire::ReadOnly)
}

/// Values that end up in a harness' argv must not be readable as options.
fn validate_args(model: &str, allowed_tools: &[String]) -> Result<(), ApiError> {
    workspace::validate_arg_value("model", model)?;
    for t in allowed_tools {
        workspace::validate_arg_value("allowed tool", t)?;
    }
    Ok(())
}

fn caps_check(h: &dyn Harness, what: &str, needs: &Needs) -> Result<(), ApiError> {
    check_caps(h, what, needs).map_err(|e| ApiError::new(ErrorCode::CapabilityMissing, e.to_string()))
}

/// A one-shot runs outside the run engine, so it has its own bound: at most `max_concurrency` at a
/// time (`queue_full` beyond) and `oneshot_timeout` of waiting. The harness call itself cannot be
/// cancelled from here, so a timed-out call keeps its slot until the harness returns: stuck calls
/// can never exceed the cap.
async fn oneshot(State(s): State<SharedState>, body: Bytes) -> HttpResult<Json<OneShotResponse>> {
    let req: OneShotRequest = parse(&body)?;
    let permit = s
        .oneshot_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::new(ErrorCode::QueueFull, "too many one-shot calls are already running"))?;
    let timeout = s.oneshot_timeout;
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        oneshot_blocking(&s, req)
    });
    match tokio::time::timeout(timeout, task).await {
        Ok(Ok(r)) => Ok(Json(r.map_err(HttpError)?)),
        Ok(Err(e)) => Err(HttpError(ApiError::internal(e))),
        Err(_) => Err(HttpError(ApiError::new(ErrorCode::Timeout, format!("the harness did not answer within {}s", timeout.as_secs())))),
    }
}

fn oneshot_blocking(s: &Shared, req: OneShotRequest) -> Result<OneShotResponse, ApiError> {
    {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let h = s.harness(&req.harness)?;
        validate_args(&req.model, &req.allowed_tools)?;
        caps_check(h, "this call", &needs_of(req.access.access(), req.tools))?;
        let cwd = match &req.workspace {
            Some(w) => s.resolve_workspace(w, read_only_of(req.access, req.tools))?,
            None => std::env::temp_dir(),
        };
        let tag = format!("remote-oneshot-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
        let one = OneShot {
            model: &req.model,
            system_prompt: (!req.system_prompt.is_empty()).then_some(SystemPrompt::Text(&req.system_prompt)),
            system_prompt_mode: if req.system_prompt_replace { SystemPromptMode::Replace } else { SystemPromptMode::Append },
            file_tag: &tag,
            prompt: &req.prompt,
            cwd: &cwd,
            extra_dirs: Vec::new(),
            tools: req.tools.policy(&req.allowed_tools),
            access: req.access.access(),
            session: SessionMode::Ephemeral,
            max_turns: req.max_turns,
            effort: None,
            env: Vec::new(),
            extra_args: Vec::new(),
            usage_tag: RunTag::new("remote_oneshot").harness(h.id()),
        };
        Ok(match h.one_shot(&one) {
            Ok(t) => OneShotResponse {
                text: t.text,
                usage: Some(OneShotUsage { input_tokens: t.input_tokens, output_tokens: t.output_tokens, cost_usd: t.cost_usd }),
                error: None,
            },
            Err(e) => OneShotResponse { text: String::new(), usage: None, error: Some(e.to_string()) },
        })
    }
}

fn create_run_blocking(s: &Shared, req: RunRequest) -> Result<(StatusCode, Value), ApiError> {
    if req.idempotency_key.trim().is_empty() {
        return Err(ApiError::bad_request("idempotency_key is required"));
    }
    if req.prompt.trim().is_empty() {
        return Err(ApiError::bad_request("prompt is empty"));
    }
    // The attachment bytes are written to disk, never stored in the row (nor compared on a retry).
    let stored = RunRequest { attachments: Vec::new(), ..req.clone() };
    let stored_json = serde_json::to_string(&stored).map_err(ApiError::internal)?;
    // A retry of the same request returns the first run; the same key with another body is a client bug.
    let existing = |id: i64| -> Result<(StatusCode, Value), ApiError> {
        if s.db.status(id).as_deref() == Some("preparing") {
            return Err(ApiError::new(ErrorCode::Conflict, "this request is still being prepared; retry shortly"));
        }
        if s.db.request_json(id)?.as_deref() != Some(stored_json.as_str()) {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                format!("idempotency key `{}` was already used for a different request (run {id})", req.idempotency_key),
            ));
        }
        Ok((StatusCode::OK, json!({ "id": id, "created": false })))
    };
    if let Some(id) = s.db.run_by_key(&req.idempotency_key) {
        return existing(id);
    }
    let h = s.harness(&req.harness)?;
    validate_args(&req.model, &req.allowed_tools)?;
    caps_check(h, "this run", &needs_of(req.access.access(), req.tools))?;
    let engine = &s.engine;
    let max_queue = s.max_queue.load(Ordering::Relaxed);
    if engine.active_count() as i64 >= max_queue {
        return Err(ApiError::new(ErrorCode::QueueFull, format!("{max_queue} runs are already queued or running")));
    }
    let files = workspace::decode_attachments(&req.attachments)?;
    let cwd = s.resolve_workspace(&req.workspace, read_only_of(req.access, req.tools))?;
    let (id, created) = s
        .db
        .insert_run(&req.idempotency_key, h.id(), &req.model, &stored_json, &cwd, req.timeout_secs.max(1))
        .map_err(ApiError::from)?;
    if !created {
        return existing(id);
    }
    // The row is `preparing` until the attachments are on disk, so a concurrent pump cannot start
    // the run without them.
    if let Err(e) = workspace::write_attachments(&s.scratch(id), &files) {
        s.db.delete_run(id);
        let _ = std::fs::remove_dir_all(s.scratch(id));
        return Err(ApiError::internal(format!("failed to store the attachments: {e}")));
    }
    if !s.db.mark_queued(id) {
        let _ = std::fs::remove_dir_all(s.scratch(id));
        return Err(ApiError::internal("the run vanished while it was being prepared"));
    }
    engine.submit(id);
    Ok((StatusCode::CREATED, json!({ "id": id, "created": true })))
}

async fn create_run(State(s): State<SharedState>, body: Bytes) -> HttpResult<(StatusCode, Json<Value>)> {
    let req: RunRequest = parse(&body)?;
    let (status, v) = blocking(move || create_run_blocking(&s, req)).await?;
    Ok((status, Json(v)))
}

#[derive(Deserialize)]
struct SinceQuery {
    since: Option<String>,
    limit: Option<i64>,
}

async fn list_runs(State(s): State<SharedState>, Query(q): Query<SinceQuery>) -> HttpResult<Json<Value>> {
    let runs = s.db.list_runs(q.since.as_deref(), q.limit.unwrap_or(200).clamp(1, 1000))?;
    Ok(Json(json!({ "runs": runs })))
}

fn run_or_404(s: &Shared, id: i64) -> Result<super::store::RunRow, ApiError> {
    s.db.get_run(id)?.ok_or_else(|| ApiError::new(ErrorCode::NotFound, format!("no run {id}")))
}

async fn get_run(State(s): State<SharedState>, UrlPath(id): UrlPath<i64>) -> HttpResult<Json<Value>> {
    Ok(Json(json!(run_or_404(&s, id)?)))
}

#[derive(Deserialize)]
struct EventsQuery {
    after_seq: Option<i64>,
}

/// SSE: replay stored events after `after_seq` (or `Last-Event-ID`), then follow live, and end with a
/// terminal `status` event once the run is over and every event was sent.
async fn run_events(
    State(s): State<SharedState>,
    UrlPath(id): UrlPath<i64>,
    Query(q): Query<EventsQuery>,
    headers: HeaderMap,
) -> HttpResult<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>> {
    run_or_404(&s, id)?;
    let mut after = q
        .after_seq
        .or_else(|| headers.get("last-event-id").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()))
        .unwrap_or(0);
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(64);
    // Subscribe before the first read so a change in between is not missed.
    let mut changes = s.changes.subscribe();
    tokio::spawn(async move {
        const BATCH: i64 = 500;
        loop {
            // The status is read before the events: every event of a finished run is stored before
            // its status flips, so a terminal status followed by an empty read means "all sent".
            let Ok(Some(run)) = s.db.get_run(id) else { return };
            let Ok(events) = s.db.events_after(id, after, BATCH) else { return };
            let n = events.len() as i64;
            for e in events {
                after = e.seq;
                let data = json!({ "seq": e.seq, "kind": e.kind, "json": e.json, "norm": e.norm }).to_string();
                if tx.send(Ok(Event::default().event("event").id(e.seq.to_string()).data(data))).await.is_err() {
                    return;
                }
            }
            if n == BATCH {
                continue;
            }
            if is_terminal(&run.status) {
                if n > 0 {
                    continue;
                }
                let data = json!({
                    "status": run.status, "error": run.error, "text": run.text, "usage": run.usage, "last_seq": after,
                })
                .to_string();
                let _ = tx.send(Ok(Event::default().event("status").data(data))).await;
                return;
            }
            tokio::select! {
                _ = changes.recv() => {}
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                _ = tx.closed() => return,
            }
        }
    });
    Ok(Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default()))
}

async fn stop_run(State(s): State<SharedState>, UrlPath(id): UrlPath<i64>) -> HttpResult<(StatusCode, Json<Value>)> {
    blocking(move || {
        let run = run_or_404(&s, id)?;
        if is_terminal(&run.status) {
            return Err(ApiError::bad_request(format!("run {id} is already {}", run.status)));
        }
        s.engine.stop(id).map_err(|e| ApiError::bad_request(e.to_string()))?;
        // A run stopped while queued never launched, so nothing else removes its attachments.
        if s.db.status(id).as_deref() == Some("killed") {
            let _ = std::fs::remove_dir_all(s.scratch(id));
        }
        Ok((StatusCode::ACCEPTED, Json(json!({ "id": id, "stopping": true }))))
    })
    .await
}

async fn sessions(State(s): State<SharedState>) -> HttpResult<Json<Value>> {
    blocking(move || {
        let mut out = Vec::new();
        for h in &s.harnesses {
            for session in h.live_sessions() {
                let mut v = serde_json::to_value(&session).map_err(ApiError::internal)?;
                if let Some(o) = v.as_object_mut() {
                    o.insert("harness".into(), json!(h.id()));
                }
                out.push(v);
            }
        }
        Ok(Json(json!({ "sessions": out })))
    })
    .await
}

#[derive(Deserialize)]
struct OutboxQuery {
    after: Option<i64>,
}

async fn outbox(State(s): State<SharedState>, UrlPath(id): UrlPath<i64>, Query(q): Query<OutboxQuery>) -> HttpResult<Json<Value>> {
    run_or_404(&s, id)?;
    Ok(Json(json!({ "ops": s.db.outbox_after(id, q.after.unwrap_or(0))? })))
}

#[derive(Deserialize)]
struct AckBody {
    upto: i64,
}

async fn outbox_ack(State(s): State<SharedState>, UrlPath(id): UrlPath<i64>, body: Bytes) -> HttpResult<Json<Value>> {
    run_or_404(&s, id)?;
    let ack: AckBody = parse(&body)?;
    Ok(Json(json!({ "acked": s.db.ack_outbox(id, ack.upto)? })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{Access, Caps, Event as AiEvent, Interactive, StreamRun, TurnResult};
    use crate::error::AppError;
    use std::net::SocketAddr;
    use std::process::{Child, Command, Stdio};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Runs `sh -c <model>` as a group leader; `EXTRA` names the run's first extra directory.
    struct Fake;

    impl Harness for Fake {
        fn id(&self) -> &'static str {
            "fake"
        }
        fn name(&self) -> &'static str {
            "Fake"
        }
        fn caps(&self) -> Caps {
            Caps {
                sessions: false,
                resume: false,
                fork: false,
                tools: true,
                tool_allowlist: false,
                streaming: true,
                interactive_tui: false,
                reports_cost: false,
                replaces_system_prompt: false,
                access_levels: vec![Access::ReadOnly, Access::Edit],
                pins_session_id: false,
            }
        }
        fn available(&self) -> bool {
            true
        }
        fn one_shot(&self, req: &OneShot<'_>) -> AppResult<TurnResult> {
            if req.prompt == "slow" {
                std::thread::sleep(Duration::from_millis(1500));
            }
            if req.prompt == "fail" {
                return Err(AppError::Other("the harness failed".into()));
            }
            Ok(TurnResult { text: format!("echo:{}@{}", req.prompt, req.cwd.display()), input_tokens: 3, output_tokens: 4, cost_usd: 0.5 })
        }
        fn spawn(&self, run: StreamRun<'_>) -> AppResult<Child> {
            use std::os::unix::process::CommandExt;
            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg(run.req.model).current_dir(run.req.cwd);
            if let Some(d) = run.req.extra_dirs.first() {
                cmd.env("EXTRA", d);
            }
            for (k, v) in &run.req.env {
                cmd.env(k, v);
            }
            cmd.stdin(Stdio::piped()).stdout(Stdio::from(run.stdout)).stderr(Stdio::from(run.stderr));
            cmd.process_group(0);
            cmd.spawn().map_err(|e| AppError::Other(e.to_string()))
        }
        fn parse_line(&self, line: &str) -> (String, Option<Value>) {
            match serde_json::from_str::<Value>(line) {
                Ok(v) => (v.get("type").and_then(Value::as_str).unwrap_or("unknown").to_string(), Some(v)),
                Err(_) => ("raw".to_string(), None),
            }
        }
        fn normalize(&self, line: &str) -> Vec<AiEvent> {
            match serde_json::from_str::<Value>(line) {
                Ok(v) if v["type"] == "result" => vec![AiEvent::Final {
                    ok: true,
                    text: v["result"].as_str().unwrap_or_default().to_string(),
                    usage: Default::default(),
                }],
                Ok(v) => vec![AiEvent::Other { kind: "x".into(), raw: v }],
                Err(_) => vec![AiEvent::Other { kind: "raw".into(), raw: Value::Null }],
            }
        }
        fn is_own_process(&self, _: i32, _: Option<&str>) -> bool {
            false
        }
        fn interactive_argv(&self, _: &Interactive<'_>) -> Option<Vec<String>> {
            None
        }
    }

    fn git(dir: &Path, args: &[&str]) {
        let o = Command::new("git").current_dir(dir).args(args).output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    }

    struct Server {
        addr: SocketAddr,
        state: SharedState,
        repo: PathBuf,
        _dir: tempfile::TempDir,
    }

    async fn server(max_concurrency: i64) -> Server {
        server_with(max_concurrency, ONESHOT_TIMEOUT).await
    }

    async fn server_with(max_concurrency: i64, oneshot_timeout: Duration) -> Server {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("widgets");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "t@example.com"]);
        git(&repo, &["config", "user.name", "t"]);
        git(&repo, &["remote", "add", "origin", "https://github.com/acme/widgets.git"]);
        std::fs::write(repo.join("a.txt"), "a").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        let app_db = dir.path().join("app.db");
        let conn = crate::db::open(&app_db).unwrap();
        crate::commands::repo::add_repository_impl(&conn, repo.to_str().unwrap()).unwrap();
        drop(conn);
        let harness: &'static dyn Harness = Box::leak(Box::new(Fake));
        let state = build(
            Settings {
                app_db,
                serve_db_path: dir.path().join("serve.db"),
                data_dir: dir.path().to_path_buf(),
                max_concurrency,
                oneshot_timeout,
                worktree_base: dir.path().join("wt"),
                pr_head: Box::new(|_, _| Err(crate::error::AppError::Other("no gh in tests".into()))),
            },
            vec![harness],
        )
        .unwrap();
        super::super::daemon::start_engines(&state);
        let bound = super::super::daemon::Bound::bind(&crate::serve::Listen::Tcp("127.0.0.1:0".parse().unwrap())).await.unwrap();
        let addr = bound.local_addr().unwrap();
        tokio::spawn(super::super::daemon::serve(state.clone(), bound, std::future::pending()));
        Server { addr, state, repo, _dir: dir }
    }

    /// One request over a fresh connection; chunked bodies are decoded.
    async fn call<S: AsyncReadExt + AsyncWriteExt + Unpin>(mut s: S, method: &str, path: &str, body: Option<&str>) -> (u16, String) {
        let body = body.unwrap_or("");
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).await.unwrap();
        let text = String::from_utf8_lossy(&raw).to_string();
        let (head, rest) = text.split_once("\r\n\r\n").unwrap();
        let status: u16 = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        let body = if head.to_ascii_lowercase().contains("transfer-encoding: chunked") { dechunk(rest) } else { rest.to_string() };
        (status, body)
    }

    fn dechunk(mut s: &str) -> String {
        let mut out = String::new();
        while let Some((size, rest)) = s.split_once("\r\n") {
            let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
            if n == 0 {
                break;
            }
            out.push_str(&rest[..n]);
            s = &rest[n + 2..];
        }
        out
    }

    async fn req(srv: &Server, method: &str, path: &str, body: Option<&str>) -> (u16, Value) {
        let (status, text) = call(tokio::net::TcpStream::connect(srv.addr).await.unwrap(), method, path, body).await;
        (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }

    fn run_body(key: &str, script: &str) -> String {
        json!({
            "idempotency_key": key, "harness": "fake", "model": script, "prompt": "go",
            "workspace": {"repo": "acme/widgets", "kind": "main-clone"}
        })
        .to_string()
    }

    async fn create(srv: &Server, key: &str, script: &str) -> i64 {
        let (status, v) = req(srv, "POST", "/v1/runs", Some(&run_body(key, script))).await;
        assert!(status == 201 || status == 200, "{status} {v}");
        v["id"].as_i64().unwrap()
    }

    async fn wait_done(srv: &Server, id: i64) -> Value {
        for _ in 0..200 {
            let (_, v) = req(srv, "GET", &format!("/v1/runs/{id}"), None).await;
            if !matches!(v["status"].as_str(), Some("queued" | "running")) {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("run {id} did not finish");
    }

    /// `(event name, data)` pairs of an SSE body.
    fn sse(body: &str) -> Vec<(String, Value)> {
        body.split("\n\n")
            .filter_map(|block| {
                let name = block.lines().find_map(|l| l.strip_prefix("event:"))?.trim().to_string();
                let data = block.lines().find_map(|l| l.strip_prefix("data:"))?.trim();
                Some((name, serde_json::from_str(data).unwrap()))
            })
            .collect()
    }

    const OK_RUN: &str = r#"echo '{"type":"a"}'; echo '{"type":"b"}'; echo '{"type":"result","result":"done"}'"#;

    #[tokio::test(flavor = "multi_thread")]
    async fn info_reports_protocol_harnesses_and_load() {
        let srv = server(2).await;
        let (status, v) = req(&srv, "GET", "/v1/info", None).await;
        assert_eq!(status, 200);
        assert_eq!(v["protocol"], PROTOCOL);
        assert_eq!(v["version"], env!("CODEREVIEW_VERSION"));
        assert_eq!(v["max_concurrency"], 2);
        assert_eq!(v["load"], json!({"running": 0, "queued": 0}));
        assert_eq!(v["harnesses"][0]["id"], "fake");
        assert_eq!(v["harnesses"][0]["available"], true);
        assert_eq!(v["harnesses"][0]["caps"]["streaming"], true);
        let (status, v) = req(&srv, "GET", "/v1/nope", None).await;
        assert_eq!((status, v["error"]["code"].as_str()), (404, Some("not_found")));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn option_like_values_and_bad_branches_are_refused() {
        let srv = server(1).await;
        let post = |patch: Box<dyn Fn(&mut Value)>| {
            let mut v: Value = serde_json::from_str(&run_body("k-bad", "true")).unwrap();
            patch(&mut v);
            v.to_string()
        };
        let cases: Vec<Box<dyn Fn(&mut Value)>> = vec![
            Box::new(|v| v["model"] = json!("--dangerously-skip-permissions")),
            Box::new(|v| {
                v["tools"] = json!("allowlist");
                v["allowed_tools"] = json!(["Read", "--evil"]);
            }),
            Box::new(|v| v["workspace"] = json!({"repo": "acme/widgets", "kind": "branch-worktree", "branch": "--upload-pack=x"})),
            Box::new(|v| v["workspace"] = json!({"repo": "acme/widgets", "kind": "branch-worktree", "branch": "a..b"})),
        ];
        for case in cases {
            let (status, v) = req(&srv, "POST", "/v1/runs", Some(&post(case))).await;
            assert_eq!((status, v["error"]["code"].as_str()), (400, Some("bad_request")), "{v}");
        }
        // nothing was stored for a refused request
        let (_, v) = req(&srv, "GET", "/v1/runs", None).await;
        assert_eq!(v["runs"].as_array().map(Vec::len), Some(0));
        let body = json!({"harness": "fake", "prompt": "x", "model": "-m"}).to_string();
        let (status, _) = req(&srv, "POST", "/v1/oneshot", Some(&body)).await;
        assert_eq!(status, 400);
        // a branch of the main clone cannot be shared with a run that writes
        let body = json!({
            "idempotency_key": "k-main", "harness": "fake", "prompt": "go", "access": "edit", "tools": "edit",
            "workspace": {"repo": "acme/widgets", "kind": "branch-worktree", "branch": "main"}
        })
        .to_string();
        let (status, v) = req(&srv, "POST", "/v1/runs", Some(&body)).await;
        assert_eq!((status, v["error"]["code"].as_str()), (422, Some("no_worktree")), "{v}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn models_and_unknown_harness() {
        let srv = server(1).await;
        let (status, v) = req(&srv, "GET", "/v1/harnesses/fake/models", None).await;
        assert_eq!((status, v["models"].as_array().map(Vec::len)), (200, Some(0)));
        let (status, v) = req(&srv, "GET", "/v1/harnesses/ghost/models", None).await;
        assert_eq!((status, v["error"]["code"].as_str()), (422, Some("harness_unavailable")));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn oneshot_returns_text_usage_and_harness_errors() {
        let srv = server(1).await;
        let body = json!({"harness": "fake", "prompt": "hi"}).to_string();
        let (status, v) = req(&srv, "POST", "/v1/oneshot", Some(&body)).await;
        assert_eq!(status, 200);
        assert!(v["text"].as_str().unwrap().starts_with("echo:hi@"), "{v}");
        assert_eq!(v["usage"], json!({"input_tokens": 3, "output_tokens": 4, "cost_usd": 0.5}));
        assert!(v["error"].is_null());
        let body = json!({"harness": "fake", "prompt": "fail"}).to_string();
        let (status, v) = req(&srv, "POST", "/v1/oneshot", Some(&body)).await;
        assert_eq!(status, 200);
        assert_eq!(v["error"], "the harness failed");
        assert_eq!(v["text"], "");
        // with a workspace the call runs in the repo
        let body = json!({"harness": "fake", "prompt": "w", "workspace": {"repo": "acme/widgets", "kind": "main-clone"}}).to_string();
        let (_, v) = req(&srv, "POST", "/v1/oneshot", Some(&body)).await;
        assert!(v["text"].as_str().unwrap().ends_with(srv.repo.to_str().unwrap()), "{v}");
        // a capability the harness lacks is refused up front
        let body = json!({"harness": "fake", "prompt": "x", "access": "full"}).to_string();
        let (status, v) = req(&srv, "POST", "/v1/oneshot", Some(&body)).await;
        assert_eq!((status, v["error"]["code"].as_str()), (422, Some("capability_missing")));
        let (status, v) = req(&srv, "POST", "/v1/oneshot", Some("{not json")).await;
        assert_eq!((status, v["error"]["code"].as_str()), (400, Some("bad_request")));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_run_goes_queued_to_ok_and_its_events_replay_from_a_seq() {
        let srv = server(2).await;
        let id = create(&srv, "k-life", OK_RUN).await;
        let done = wait_done(&srv, id).await;
        assert_eq!(done["status"], "ok", "{done}");
        assert_eq!(done["text"], "done");
        assert_eq!(done["harness"], "fake");
        assert_eq!(done["last_seq"], 3);
        assert!(done["finished_at"].is_string() && done["started_at"].is_string());

        let (status, body) = call(tokio::net::TcpStream::connect(srv.addr).await.unwrap(), "GET", &format!("/v1/runs/{id}/events"), None).await;
        assert_eq!(status, 200);
        let events = sse(&body);
        let seqs: Vec<i64> = events.iter().filter(|(n, _)| n == "event").map(|(_, d)| d["seq"].as_i64().unwrap()).collect();
        assert_eq!(seqs, vec![1, 2, 3]);
        assert_eq!(events[0].1["json"]["type"], "a");
        assert_eq!(events[2].1["norm"]["events"][0]["type"], "final");
        let (name, last) = events.last().unwrap();
        assert_eq!((name.as_str(), last["status"].as_str(), last["text"].as_str()), ("status", Some("ok"), Some("done")));

        // replay after seq 2 sends only the third event, then the terminal status
        let (_, body) = call(tokio::net::TcpStream::connect(srv.addr).await.unwrap(), "GET", &format!("/v1/runs/{id}/events?after_seq=2"), None).await;
        let events = sse(&body);
        assert_eq!(events.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), vec!["event", "status"]);
        assert_eq!(events[0].1["seq"], 3);

        // the list reports it, and `since` filters by update stamp
        let (_, list) = req(&srv, "GET", "/v1/runs", None).await;
        assert_eq!(list["runs"][0]["id"], id);
        let stamp = done["updated_at"].as_str().unwrap();
        let (_, later) = req(&srv, "GET", &format!("/v1/runs?since={stamp}"), None).await;
        assert!(later["runs"].as_array().unwrap().is_empty());
        let (status, v) = req(&srv, "GET", "/v1/runs/999", None).await;
        assert_eq!((status, v["error"]["code"].as_str()), (404, Some("not_found")));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stream_opened_while_the_run_is_live_follows_it_to_the_end() {
        let srv = server(2).await;
        let id = create(&srv, "k-live", r#"echo '{"type":"a"}'; sleep 1; echo '{"type":"result","result":"late"}'"#).await;
        let (status, body) = call(tokio::net::TcpStream::connect(srv.addr).await.unwrap(), "GET", &format!("/v1/runs/{id}/events"), None).await;
        assert_eq!(status, 200);
        let events = sse(&body);
        assert_eq!(events.iter().filter(|(n, _)| n == "event").count(), 2, "{body}");
        assert_eq!(events.last().unwrap().1["status"], "ok");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn create_is_idempotent_on_the_key() {
        let srv = server(2).await;
        let (s1, a) = req(&srv, "POST", "/v1/runs", Some(&run_body("same", OK_RUN))).await;
        let (s2, b) = req(&srv, "POST", "/v1/runs", Some(&run_body("same", OK_RUN))).await;
        assert_eq!((s1, s2), (201, 200));
        assert_eq!(a["id"], b["id"]);
        assert_eq!((a["created"].clone(), b["created"].clone()), (json!(true), json!(false)));
        wait_done(&srv, a["id"].as_i64().unwrap()).await;
        let (_, list) = req(&srv, "GET", "/v1/runs", None).await;
        assert_eq!(list["runs"].as_array().unwrap().len(), 1);
        // the same key with another body is refused instead of silently returning the first run
        let (s3, c) = req(&srv, "POST", "/v1/runs", Some(&run_body("same", "echo different"))).await;
        assert_eq!((s3, c["error"]["code"].as_str()), (409, Some("conflict")), "{c}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_run_is_not_started_before_its_preparation_is_done() {
        let srv = server(2).await;
        let key = "k-prep";
        let (id, created) = srv
            .state
            .db
            .insert_run(key, "fake", "", &run_body(key, OK_RUN), &srv.repo, 60)
            .unwrap();
        assert!(created);
        // a pump (any other request finishing, a restart) cannot take it while it is `preparing`
        srv.state.engine.pump();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(srv.state.db.status(id).as_deref(), Some("preparing"));
        assert_eq!(srv.state.db.get_run(id).unwrap().unwrap().last_seq, 0);
        // a failed preparation removes only that row, never one the engine already owns
        assert!(srv.state.db.mark_queued(id));
        srv.state.db.delete_run(id);
        assert!(srv.state.db.status(id).is_some(), "a queued row is not deleted");
        srv.state.engine.pump();
        assert_eq!(wait_done(&srv, id).await["status"], "ok");
        // a crash between insert and queue leaves a row the next start purges
        let (stale, _) = srv.state.db.insert_run("k-stale", "fake", "", &run_body("k-stale", "true"), &srv.repo, 60).unwrap();
        assert_eq!(srv.state.db.purge_preparing(), vec![stale]);
        assert!(srv.state.db.status(stale).is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_creates_with_attachments_always_see_the_files() {
        let srv = Arc::new(server(4).await);
        let script = r#"printf '{"type":"result","result":"%s"}\n' "$(cat "$EXTRA/note.txt")""#;
        let mut tasks = Vec::new();
        for i in 0..6 {
            let srv = srv.clone();
            tasks.push(tokio::spawn(async move {
                let mut v: Value = serde_json::from_str(&run_body(&format!("race-{i}"), script)).unwrap();
                v["attachments"] = json!([{"name": "note.txt", "content_base64": "aGVsbG8="}]);
                let (status, created) = req(&srv, "POST", "/v1/runs", Some(&v.to_string())).await;
                assert_eq!(status, 201, "{created}");
                wait_done(&srv, created["id"].as_i64().unwrap()).await
            }));
        }
        for t in tasks {
            let done = t.await.unwrap();
            assert_eq!((done["status"].as_str(), done["text"].as_str()), (Some("ok"), Some("hello")), "{done}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_requests_for_one_branch_share_a_single_worktree() {
        let srv = Arc::new(server(4).await);
        git(&srv.repo, &["branch", "feat"]);
        let mut tasks = Vec::new();
        for i in 0..5 {
            let srv = srv.clone();
            tasks.push(tokio::spawn(async move {
                let mut v: Value = serde_json::from_str(&run_body(&format!("wt-{i}"), r#"printf '{"type":"result","result":"%s"}\n' "$(pwd)""#)).unwrap();
                v["workspace"] = json!({"repo": "acme/widgets", "kind": "branch-worktree", "branch": "feat"});
                let (status, created) = req(&srv, "POST", "/v1/runs", Some(&v.to_string())).await;
                assert_eq!(status, 201, "{created}");
                wait_done(&srv, created["id"].as_i64().unwrap()).await
            }));
        }
        let mut dirs = std::collections::HashSet::new();
        for t in tasks {
            let done = t.await.unwrap();
            assert_eq!(done["status"], "ok", "{done}");
            dirs.insert(done["text"].as_str().unwrap().to_string());
        }
        assert_eq!(dirs.len(), 1, "{dirs:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn oneshots_are_capped_and_time_out() {
        let srv = Arc::new(server_with(1, Duration::from_millis(300)).await);
        let slow = json!({"harness": "fake", "prompt": "slow"}).to_string();
        let quick = json!({"harness": "fake", "prompt": "hi"}).to_string();
        let first = {
            let srv = srv.clone();
            tokio::spawn(async move { req(&srv, "POST", "/v1/oneshot", Some(&slow)).await })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        // the one slot is taken: refused at once instead of piling up
        let (status, v) = req(&srv, "POST", "/v1/oneshot", Some(&quick)).await;
        assert_eq!((status, v["error"]["code"].as_str()), (429, Some("queue_full")), "{v}");
        // the slow call times out for its client ...
        let (status, v) = first.await.unwrap();
        assert_eq!((status, v["error"]["code"].as_str()), (504, Some("timeout")), "{v}");
        // ... but keeps its slot until the harness returns
        let (status, _) = req(&srv, "POST", "/v1/oneshot", Some(&quick)).await;
        assert_eq!(status, 429);
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let (status, v) = req(&srv, "POST", "/v1/oneshot", Some(&quick)).await;
        assert_eq!(status, 200, "{v}");
    }

    /// A request with exactly these header lines.
    async fn raw(srv: &Server, method: &str, path: &str, headers: &[&str], body: &str) -> (u16, Value) {
        let mut text = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\n");
        for h in headers {
            text.push_str(&format!("{h}\r\n"));
        }
        text.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
        let mut s = tokio::net::TcpStream::connect(srv.addr).await.unwrap();
        s.write_all(text.as_bytes()).await.unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).await.unwrap();
        let out = String::from_utf8_lossy(&out).to_string();
        let (head, rest) = out.split_once("\r\n\r\n").unwrap();
        let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        let rest = if head.to_ascii_lowercase().contains("transfer-encoding: chunked") { dechunk(rest) } else { rest.to_string() };
        (status, serde_json::from_str(&rest).unwrap_or(Value::String(rest)))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_tcp_listener_refuses_foreign_hosts_origins_and_non_json_posts() {
        let srv = server(1).await;
        let body = json!({"harness": "fake", "prompt": "hi"}).to_string();
        // DNS rebinding: the Host is an attacker's name
        let (s, v) = raw(&srv, "GET", "/v1/info", &["Host: evil.example.com"], "").await;
        assert_eq!((s, v["error"]["code"].as_str()), (403, Some("forbidden")), "{v}");
        let (s, _) = raw(&srv, "GET", "/v1/info", &["Host: 127.0.0.1:7777"], "").await;
        assert_eq!(s, 200);
        let (s, _) = raw(&srv, "GET", "/v1/info", &["Host: [::1]:7777"], "").await;
        assert_eq!(s, 200);
        let (s, _) = raw(&srv, "GET", "/v1/info", &["Host: 127.0.0.1.evil.example.com"], "").await;
        assert_eq!(s, 403);
        // CSRF from a web page: browsers add Origin
        let (s, v) = raw(&srv, "POST", "/v1/runs/1/stop", &["Host: localhost", "Origin: https://evil.example.com"], "").await;
        assert_eq!((s, v["error"]["code"].as_str()), (403, Some("forbidden")), "{v}");
        // a form post cannot be application/json
        for ct in ["Content-Type: text/plain", "Content-Type: application/x-www-form-urlencoded"] {
            let (s, v) = raw(&srv, "POST", "/v1/oneshot", &["Host: localhost", ct], &body).await;
            assert_eq!((s, v["error"]["code"].as_str()), (400, Some("bad_request")), "{ct}: {v}");
        }
        let (s, v) = raw(&srv, "POST", "/v1/oneshot", &["Host: localhost"], &body).await;
        assert_eq!(s, 400, "{v}");
        let (s, v) = raw(&srv, "POST", "/v1/oneshot", &["Host: localhost", "Content-Type: application/json; charset=utf-8"], &body).await;
        assert_eq!(s, 200, "{v}");
        // a body-less POST (stop) needs no content type; the run does not exist
        let (s, _) = raw(&srv, "POST", "/v1/runs/999/stop", &["Host: localhost"], "").await;
        assert_eq!(s, 404);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_unix_socket_skips_the_host_check_but_not_the_origin_one() {
        let srv = server(1).await;
        let sock = srv._dir.path().join("g.sock");
        let bound = super::super::daemon::Bound::bind(&crate::serve::Listen::Unix(sock.clone())).await.unwrap();
        tokio::spawn(super::super::daemon::serve(srv.state.clone(), bound, std::future::pending()));
        let text = |host: &str, extra: &str| format!("GET /v1/info HTTP/1.1\r\nHost: {host}\r\n{extra}Connection: close\r\nContent-Length: 0\r\n\r\n");
        for (req_text, want) in [(text("anything", ""), "200"), (text("anything", "Origin: https://x\r\n"), "403")] {
            let mut s = tokio::net::UnixStream::connect(&sock).await.unwrap();
            s.write_all(req_text.as_bytes()).await.unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).await.unwrap();
            assert!(out.starts_with(&format!("HTTP/1.1 {want}")), "{out}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stop_kills_a_running_run_and_a_queued_one() {
        let srv = server(1).await;
        let running = create(&srv, "k-run", "sleep 30").await;
        let queued = create(&srv, "k-queue", "sleep 30").await;
        for _ in 0..100 {
            let (_, v) = req(&srv, "GET", &format!("/v1/runs/{running}"), None).await;
            if v["status"] == "running" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        assert_eq!(req(&srv, "GET", &format!("/v1/runs/{queued}"), None).await.1["status"], "queued", "cap of 1");
        let (_, info) = req(&srv, "GET", "/v1/info", None).await;
        assert_eq!(info["load"], json!({"running": 1, "queued": 1}));
        let (status, _) = req(&srv, "POST", &format!("/v1/runs/{queued}/stop"), None).await;
        assert_eq!(status, 202);
        assert_eq!(wait_done(&srv, queued).await["status"], "killed");
        let (status, _) = req(&srv, "POST", &format!("/v1/runs/{running}/stop"), None).await;
        assert_eq!(status, 202);
        assert_eq!(wait_done(&srv, running).await["status"], "killed");
        let (status, v) = req(&srv, "POST", &format!("/v1/runs/{running}/stop"), None).await;
        assert_eq!((status, v["error"]["code"].as_str()), (400, Some("bad_request")));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn requests_that_cannot_run_fail_with_stable_codes() {
        let srv = server(1).await;
        let post = |body: Value| {
            let srv = &srv;
            async move { req(srv, "POST", "/v1/runs", Some(&body.to_string())).await }
        };
        let base = |f: &dyn Fn(&mut Value)| {
            let mut v: Value = serde_json::from_str(&run_body("k-err", "true")).unwrap();
            f(&mut v);
            v
        };
        let (s, v) = post(base(&|v| v["workspace"]["repo"] = json!("acme/ghost"))).await;
        assert_eq!((s, v["error"]["code"].as_str()), (422, Some("unknown_repo")));
        let (s, v) = post(base(&|v| v["harness"] = json!("ghost"))).await;
        assert_eq!((s, v["error"]["code"].as_str()), (422, Some("harness_unavailable")));
        let (s, v) = post(base(&|v| v["access"] = json!("full"))).await;
        assert_eq!((s, v["error"]["code"].as_str()), (422, Some("capability_missing")));
        let (s, v) = post(base(&|v| v["tools"] = json!("allowlist"))).await;
        assert_eq!((s, v["error"]["code"].as_str()), (422, Some("capability_missing")));
        let (s, v) = post(base(&|v| v["prompt"] = json!("  "))).await;
        assert_eq!((s, v["error"]["code"].as_str()), (400, Some("bad_request")));
        let (s, v) = post(base(&|v| v["workspace"] = json!({"repo": "acme/widgets", "kind": "branch-worktree"}))).await;
        assert_eq!((s, v["error"]["code"].as_str()), (400, Some("bad_request")));
        let (s, v) = post(base(&|v| v["workspace"] = json!({"repo": "acme/widgets", "kind": "pr-worktree", "pr": 4}))).await;
        assert_eq!((s, v["error"]["code"].as_str()), (422, Some("no_worktree")));
        let (s, v) = post(base(&|v| v["attachments"] = json!([{"name": "../x", "content_base64": "eA=="}]))).await;
        assert_eq!((s, v["error"]["code"].as_str()), (400, Some("bad_request")));
        // nothing was created by any of them
        assert!(req(&srv, "GET", "/v1/runs", None).await.1["runs"].as_array().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_full_queue_is_refused() {
        let srv = server(1).await;
        srv.state.max_queue.store(1, Ordering::Relaxed);
        let small = &srv;
        let first = create(small, "q1", "sleep 30").await;
        let (s, v) = req(small, "POST", "/v1/runs", Some(&run_body("q2", "true"))).await;
        assert_eq!((s, v["error"]["code"].as_str()), (429, Some("queue_full")));
        req(small, "POST", &format!("/v1/runs/{first}/stop"), None).await;
        wait_done(small, first).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn attachments_and_the_remote_environment_reach_the_run() {
        let srv = server(1).await;
        let script = r#"printf '{"type":"result","result":"%s|%s|%s|%s"}\n' "$(cat "$EXTRA/note.txt")" "$CODEREVIEW_REMOTE_RUN" "$(cat "$CODEREVIEW_REMOTE_DIR/context.json" | tr -d ' "')" "$(pwd)""#;
        let mut v: Value = serde_json::from_str(&run_body("k-att", script)).unwrap();
        v["attachments"] = json!([{"name": "note.txt", "content_base64": "aGVsbG8="}]);
        v["context"] = json!({"cards": []});
        let (status, created) = req(&srv, "POST", "/v1/runs", Some(&v.to_string())).await;
        assert_eq!(status, 201, "{created}");
        let id = created["id"].as_i64().unwrap();
        let done = wait_done(&srv, id).await;
        assert_eq!(done["status"], "ok", "{done}");
        let text = done["text"].as_str().unwrap();
        let parts: Vec<&str> = text.split('|').collect();
        assert_eq!(parts[0], "hello");
        assert_eq!(parts[1], id.to_string());
        assert_eq!(parts[2], "{cards:[]}");
        assert_eq!(Path::new(parts[3]).canonicalize().unwrap(), srv.repo.canonicalize().unwrap());
        // the scratch dir goes with the run
        assert!(!srv.state.runs_base.join("scratch").join(id.to_string()).exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_outbox_lists_ops_after_a_cursor_and_prunes_on_ack() {
        let srv = server(1).await;
        let id = create(&srv, "k-out", OK_RUN).await;
        wait_done(&srv, id).await;
        let remote = crate::serve::remote::RemoteRun { run_id: id, db: srv.state.serve_db_path.clone(), dir: PathBuf::new() };
        remote.append("board_link", json!({"issue_id": "i1", "pr_reference": "acme/widgets#2"})).unwrap();
        remote.append("notify", json!({"message": "m"})).unwrap();
        let (_, v) = req(&srv, "GET", &format!("/v1/runs/{id}/outbox"), None).await;
        let ops = v["ops"].as_array().unwrap();
        assert_eq!(ops.iter().map(|o| o["type"].as_str().unwrap()).collect::<Vec<_>>(), vec!["board_link", "notify"]);
        assert_eq!(ops[0]["op_id"], format!("{id}:1"));
        let (_, v) = req(&srv, "GET", &format!("/v1/runs/{id}/outbox?after=1"), None).await;
        assert_eq!(v["ops"].as_array().unwrap().len(), 1);
        let (status, v) = req(&srv, "POST", &format!("/v1/runs/{id}/outbox/ack"), Some(r#"{"upto":2}"#)).await;
        assert_eq!((status, v["acked"].clone()), (200, json!(2)));
        let (_, v) = req(&srv, "GET", &format!("/v1/runs/{id}/outbox"), None).await;
        assert!(v["ops"].as_array().unwrap().is_empty());
        let (status, _) = req(&srv, "POST", &format!("/v1/runs/{id}/outbox/ack"), Some("nope")).await;
        assert_eq!(status, 400);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sessions_lists_live_sessions_of_the_harnesses() {
        let srv = server(1).await;
        let (status, v) = req(&srv, "GET", "/v1/sessions", None).await;
        assert_eq!((status, v["sessions"].as_array().map(Vec::len)), (200, Some(0)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_api_is_reachable_over_a_unix_socket() {
        let srv = server(1).await;
        let sock = srv._dir.path().join("run").join("serve.sock");
        let bound = super::super::daemon::Bound::bind(&crate::serve::Listen::Unix(sock.clone())).await.unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&sock).unwrap().permissions().mode() & 0o777, 0o600);
        tokio::spawn(super::super::daemon::serve(srv.state.clone(), bound, std::future::pending()));
        let (status, body) = call(tokio::net::UnixStream::connect(&sock).await.unwrap(), "GET", "/v1/info", None).await;
        assert_eq!(status, 200);
        assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["protocol"], PROTOCOL);
        // the blocking probe used by `cr serve --status` sees the same
        let listen = crate::serve::Listen::Unix(sock.clone());
        let info = tokio::task::spawn_blocking(move || super::super::install::probe_info(&listen)).await.unwrap().unwrap();
        assert_eq!(info["protocol"], PROTOCOL);
        // a second daemon on a live socket is refused
        assert!(super::super::daemon::Bound::bind(&crate::serve::Listen::Unix(sock)).await.is_err());
    }
}
