//! `cr serve`: a long-lived daemon that exposes the local harnesses of its machine over a versioned
//! HTTP+SSE API (ADR 0007). It is a plain executor: it receives rendered prompts and neutral options,
//! runs them with the shared [`run_engine`](crate::run_engine) and keeps its own run store.
//!
//! Everything except the HTTP layer ([`http`], [`daemon`], behind the `serve` cargo feature) is plain
//! Rust: the store, the wire types, the workspace resolver, the outbox that `cr` writes to inside a
//! remote run, and the systemd helpers.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use serde::Serialize;

use crate::error::{AppError, AppResult};

#[cfg(all(feature = "serve", unix))]
pub mod daemon;
#[cfg(all(feature = "serve", unix))]
pub mod http;
pub mod install;
pub mod remote;
pub mod request;
pub mod sink;
pub mod store;
pub mod workspace;

/// Integer protocol version of `/v1`; a client with another number must not use the host.
pub const PROTOCOL: u32 = 1;

/// Environment variable naming the serve DB; also the `--serve-db` flag's env.
pub const SERVE_DB_ENV: &str = "CODEREVIEW_SERVE_DB";

/// Stable machine codes of an API error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    UnknownRepo,
    NoWorktree,
    HarnessUnavailable,
    CapabilityMissing,
    QueueFull,
    Conflict,
    Timeout,
    BadRequest,
    NotFound,
    Internal,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::UnknownRepo => "unknown_repo",
            ErrorCode::NoWorktree => "no_worktree",
            ErrorCode::HarnessUnavailable => "harness_unavailable",
            ErrorCode::CapabilityMissing => "capability_missing",
            ErrorCode::QueueFull => "queue_full",
            ErrorCode::Conflict => "conflict",
            ErrorCode::Timeout => "timeout",
            ErrorCode::BadRequest => "bad_request",
            ErrorCode::NotFound => "not_found",
            ErrorCode::Internal => "internal",
        }
    }

    pub fn http_status(self) -> u16 {
        match self {
            ErrorCode::BadRequest => 400,
            ErrorCode::NotFound => 404,
            ErrorCode::UnknownRepo | ErrorCode::NoWorktree | ErrorCode::HarnessUnavailable | ErrorCode::CapabilityMissing => 422,
            ErrorCode::QueueFull => 429,
            ErrorCode::Conflict => 409,
            ErrorCode::Timeout => 504,
            ErrorCode::Internal => 500,
        }
    }
}

/// An error of the API: a stable code plus a message for humans.
#[derive(Debug, Clone, Serialize)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::BadRequest, message)
    }

    pub fn internal(e: impl std::fmt::Display) -> Self {
        Self::new(ErrorCode::Internal, e.to_string())
    }
}

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        Self::internal(e)
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        Self::internal(e)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

/// Where the daemon listens. Never a non-loopback address (ADR 0007).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Listen {
    Unix(PathBuf),
    Tcp(SocketAddr),
}

impl std::fmt::Display for Listen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Listen::Unix(p) => write!(f, "unix:{}", p.display()),
            Listen::Tcp(a) => write!(f, "{a}"),
        }
    }
}

/// `unix:/path`, an absolute path, `127.0.0.1:PORT`, `localhost:PORT`, `[::1]:PORT` or `:PORT`.
pub fn parse_listen(s: &str) -> AppResult<Listen> {
    let s = s.trim();
    if let Some(p) = s.strip_prefix("unix:") {
        return Ok(Listen::Unix(PathBuf::from(p)));
    }
    if s.starts_with('/') {
        return Ok(Listen::Unix(PathBuf::from(s)));
    }
    let refuse = || AppError::Other(format!("`{s}`: cr serve listens on a unix socket or a loopback port only (use an SSH tunnel to reach it)"));
    let addr_text = if let Some(port) = s.strip_prefix(':') { format!("127.0.0.1:{port}") } else { s.replacen("localhost", "127.0.0.1", 1) };
    let addr: SocketAddr = addr_text.parse().map_err(|_| AppError::Other(format!("`{s}` is not a socket path or a loopback host:port")))?;
    match addr.ip() {
        IpAddr::V4(ip) if ip.is_loopback() => Ok(Listen::Tcp(addr)),
        IpAddr::V6(ip) if ip.is_loopback() => Ok(Listen::Tcp(addr)),
        _ => Err(refuse()),
    }
}

/// Default socket: `$XDG_RUNTIME_DIR/codereview/serve.sock`, else next to the app data.
pub fn default_socket() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir).join("codereview").join("serve.sock"),
        _ => crate::db_path::app_data_dir().join("serve.sock"),
    }
}

/// The serve DB: the flag, else `CODEREVIEW_SERVE_DB`, else a file of its own in the app data dir.
pub fn resolve_serve_db(cli_override: Option<&str>) -> PathBuf {
    if let Some(p) = cli_override {
        return PathBuf::from(p);
    }
    if let Some(p) = std::env::var_os(SERVE_DB_ENV) {
        return PathBuf::from(p);
    }
    crate::db_path::app_data_dir().join(if cfg!(debug_assertions) { "codereview-serve-dev.db" } else { "codereview-serve.db" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listen_accepts_sockets_and_loopback_only() {
        assert_eq!(parse_listen("/run/user/1/cr.sock").unwrap(), Listen::Unix("/run/user/1/cr.sock".into()));
        assert_eq!(parse_listen("unix:rel.sock").unwrap(), Listen::Unix("rel.sock".into()));
        assert_eq!(parse_listen("127.0.0.1:7777").unwrap(), Listen::Tcp("127.0.0.1:7777".parse().unwrap()));
        assert_eq!(parse_listen("localhost:7777").unwrap(), Listen::Tcp("127.0.0.1:7777".parse().unwrap()));
        assert_eq!(parse_listen(":7777").unwrap(), Listen::Tcp("127.0.0.1:7777".parse().unwrap()));
        assert_eq!(parse_listen("[::1]:7777").unwrap(), Listen::Tcp("[::1]:7777".parse().unwrap()));
        assert!(parse_listen("0.0.0.0:7777").unwrap_err().to_string().contains("loopback"));
        assert!(parse_listen("192.168.1.2:80").is_err());
        assert!(parse_listen("nonsense").is_err());
    }

    #[test]
    fn error_codes_are_stable() {
        assert_eq!(ErrorCode::UnknownRepo.as_str(), "unknown_repo");
        let v = serde_json::to_value(ApiError::new(ErrorCode::QueueFull, "full")).unwrap();
        assert_eq!(v["code"], "queue_full");
        assert_eq!(ErrorCode::QueueFull.http_status(), 429);
    }
}
