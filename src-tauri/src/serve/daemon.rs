//! Starting the daemon: bind the listener (unix socket or loopback port), start the run engine
//! (re-adopting runs a previous daemon left alive), serve until a signal, then detach.

use std::future::Future;
use std::path::{Path, PathBuf};

use tokio::net::{TcpListener, UnixListener, UnixStream};

use super::http::{self, SharedState};
use super::store::is_terminal;
use super::{Listen, PROTOCOL};
use crate::error::{AppError, AppResult};

pub struct Config {
    pub listen: Listen,
    /// The interactive app DB (repo registry).
    pub app_db: PathBuf,
    pub serve_db: PathBuf,
    pub max_concurrency: i64,
    pub log_level: Option<String>,
}

pub enum Bound {
    Tcp(TcpListener),
    Unix(UnixListener, PathBuf),
}

impl Bound {
    pub async fn bind(listen: &Listen) -> AppResult<Bound> {
        match listen {
            Listen::Tcp(addr) => Ok(Bound::Tcp(TcpListener::bind(addr).await?)),
            Listen::Unix(path) => {
                use std::os::unix::fs::{FileTypeExt, PermissionsExt};
                if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                    if !parent.exists() {
                        std::fs::create_dir_all(parent)?;
                        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
                    }
                }
                // Never delete anything but a stale socket: `--listen` may be a typo for a real file.
                match std::fs::symlink_metadata(path) {
                    Ok(meta) if meta.file_type().is_socket() => {
                        // A live daemon answers; a socket nobody listens on is left by a crash.
                        if UnixStream::connect(path).await.is_ok() {
                            return Err(AppError::Other(format!("another cr serve is already listening on {}", path.display())));
                        }
                        std::fs::remove_file(path)?;
                    }
                    Ok(_) => {
                        return Err(AppError::Other(format!("{} exists and is not a socket; refusing to replace it", path.display())));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
                let listener = UnixListener::bind(path)?;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
                Ok(Bound::Unix(listener, path.clone()))
            }
        }
    }

    pub fn local_addr(&self) -> Option<std::net::SocketAddr> {
        match self {
            Bound::Tcp(l) => l.local_addr().ok(),
            Bound::Unix(..) => None,
        }
    }
}

/// Serve the API on `bound` until `shutdown` completes. Open event streams do not hold it back.
pub async fn serve(state: SharedState, bound: Bound, shutdown: impl Future<Output = ()>) -> std::io::Result<()> {
    let app = http::router(state, matches!(bound, Bound::Tcp(_)));
    match bound {
        Bound::Tcp(l) => {
            tokio::select! { r = axum::serve(l, app) => r, _ = shutdown => Ok(()) }
        }
        Bound::Unix(l, path) => {
            let r = tokio::select! { r = axum::serve(l, app) => r, _ = shutdown => Ok(()) };
            let _ = std::fs::remove_file(path);
            r
        }
    }
}

/// Start the engine: timeouts, re-adoption of runs left alive by an earlier daemon, queued runs,
/// and the sweep of attachment dirs whose run is over.
pub fn start_engine(state: &SharedState) {
    // Rows a crashed daemon left half-created never became runs; their scratch goes with the sweep.
    state.db.purge_preparing();
    sweep_scratch(state);
    state.engine.start_ticker();
    state.engine.adopt_detached();
    state.engine.pump();
}

/// Remove scratch directories of runs that are over or gone (a queued run stopped before it launched).
pub fn sweep_scratch(state: &SharedState) {
    let Ok(entries) = std::fs::read_dir(state.runs_base.join("scratch")) else { return };
    for entry in entries.flatten() {
        let id = entry.file_name().to_str().and_then(|n| n.parse::<i64>().ok());
        let over = match id.and_then(|id| state.db.status(id)) {
            Some(status) => is_terminal(&status),
            None => true,
        };
        if over {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Exiting: the runs keep going and the next daemon re-adopts them.
pub fn detach(state: &SharedState) {
    state.engine.detach_all();
}

async fn wait_for_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else {
        std::future::pending::<()>().await;
        return;
    };
    tokio::select! { _ = term.recv() => {}, _ = int.recv() => {} }
}

/// `cr serve`: run the daemon until SIGTERM / SIGINT.
pub fn run(cfg: Config) -> AppResult<()> {
    crate::path_env::ensure_login_path();
    crate::tools::init();
    let data_dir = cfg.serve_db.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    if let Err(e) = crate::logging::init(&data_dir) {
        eprintln!("warning: cannot open the log file in {}: {e}", data_dir.display());
    }
    if let Some(level) = cfg.log_level.as_deref() {
        match crate::logging::parse_level(level) {
            Some(l) => log::set_max_level(l),
            None => return Err(AppError::Other(format!("unknown log level `{level}`"))),
        }
    }
    let state = http::build(
        http::Settings {
            app_db: cfg.app_db.clone(),
            serve_db_path: cfg.serve_db.clone(),
            data_dir,
            max_concurrency: cfg.max_concurrency,
            oneshot_timeout: http::ONESHOT_TIMEOUT,
            worktree_base: crate::worktree::worktrees_base(),
            pr_head: Box::new(super::workspace::gh_pr_head),
        },
        crate::ai::harnesses().to_vec(),
    )?;
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(async {
        let bound = Bound::bind(&cfg.listen).await?;
        start_engine(&state);
        eprintln!(
            "cr serve: protocol {PROTOCOL}, listening on {} (app db {}, serve db {})",
            cfg.listen,
            cfg.app_db.display(),
            cfg.serve_db.display()
        );
        log::info!("cr serve listening on {}", cfg.listen);
        serve(state.clone(), bound, wait_for_signal()).await?;
        detach(&state);
        eprintln!("cr serve: stopped, runs left alive are re-adopted at the next start");
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bind_only_ever_removes_a_stale_socket() {
        let dir = tempfile::tempdir().unwrap();
        // a regular file, a directory and a symlink at the listen path are never touched
        let file = dir.path().join("precious.db");
        std::fs::write(&file, "data").unwrap();
        let err = Bound::bind(&Listen::Unix(file.clone())).await.err().expect("must refuse").to_string();
        assert!(err.contains("not a socket"), "{err}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "data");
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        assert!(Bound::bind(&Listen::Unix(sub.clone())).await.is_err());
        assert!(sub.is_dir());
        let link = dir.path().join("link.sock");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(Bound::bind(&Listen::Unix(link.clone())).await.is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "data");
        // a socket nobody listens on (a crashed daemon) is replaced
        let sock = dir.path().join("serve.sock");
        drop(UnixListener::bind(&sock).unwrap());
        assert!(sock.exists());
        let bound = Bound::bind(&Listen::Unix(sock.clone())).await.unwrap();
        // while a live one is respected
        assert!(Bound::bind(&Listen::Unix(sock.clone())).await.is_err());
        drop(bound);
    }
}
