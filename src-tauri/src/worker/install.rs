//! `cr worker --install / --uninstall / --status`: a systemd user unit for the daemon.
//!
//! The unit uses `KillMode=process` so that stopping or restarting the service leaves the harness
//! processes of live runs alone; the next daemon re-adopts them. macOS (launchd) is documented in the
//! README, not automated.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::Listen;
use crate::error::{AppError, AppResult};

pub const UNIT_NAME: &str = "cr-worker.service";

/// What the installed unit runs.
#[derive(Debug, Clone)]
pub struct UnitSpec {
    pub exe: PathBuf,
    pub listen: Listen,
    pub app_db: Option<PathBuf>,
    pub worker_db: Option<PathBuf>,
    pub max_concurrency: Option<i64>,
    pub log_level: Option<String>,
}

fn quote(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-:=@%+,".contains(c)) {
        s.to_string()
    } else {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// The text of the unit file.
pub fn unit_text(spec: &UnitSpec) -> String {
    let mut args: Vec<String> = Vec::new();
    // `--db` is a global flag of `cr`, so it goes before the subcommand.
    if let Some(db) = &spec.app_db {
        args.push("--db".into());
        args.push(db.display().to_string());
    }
    args.push("worker".into());
    args.push("--listen".into());
    args.push(spec.listen.to_string());
    if let Some(db) = &spec.worker_db {
        args.push("--worker-db".into());
        args.push(db.display().to_string());
    }
    if let Some(n) = spec.max_concurrency {
        args.push("--max-concurrency".into());
        args.push(n.to_string());
    }
    if let Some(l) = &spec.log_level {
        args.push("--log-level".into());
        args.push(l.clone());
    }
    let exec: Vec<String> = std::iter::once(spec.exe.display().to_string()).chain(args).map(|a| quote(&a)).collect();
    format!(
        "[Unit]\nDescription=CodeReview agent daemon (cr worker)\nAfter=network.target\n\n\
         [Service]\nType=simple\nExecStart={}\nRestart=on-failure\nRestartSec=3\n\
         # Stopping the daemon must not kill the runs it started: the next daemon re-adopts them.\n\
         KillMode=process\n\n\
         [Install]\nWantedBy=default.target\n",
        exec.join(" ")
    )
}

pub fn unit_path() -> AppResult<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => dirs::home_dir().ok_or_else(|| AppError::Other("no home directory".into()))?.join(".config"),
    };
    Ok(base.join("systemd").join("user").join(UNIT_NAME))
}

fn systemctl(args: &[&str]) -> AppResult<std::process::Output> {
    Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
        .map_err(|e| AppError::Other(format!("cannot run systemctl (is systemd available?): {e}")))
}

fn systemctl_ok(args: &[&str]) -> AppResult<()> {
    let out = systemctl(args)?;
    if out.status.success() {
        return Ok(());
    }
    Err(AppError::Other(format!("systemctl --user {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim())))
}

/// Write the unit, reload systemd and enable + start the service.
pub fn install(spec: &UnitSpec) -> AppResult<PathBuf> {
    let path = unit_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, unit_text(spec))?;
    systemctl_ok(&["daemon-reload"])?;
    systemctl_ok(&["enable", "--now", UNIT_NAME])?;
    Ok(path)
}

/// Stop and disable the service and remove the unit.
pub fn uninstall() -> AppResult<()> {
    let path = unit_path()?;
    if path.exists() {
        // Not enabled / not running is fine: the unit goes away either way.
        let _ = systemctl(&["disable", "--now", UNIT_NAME]);
        std::fs::remove_file(&path)?;
        systemctl_ok(&["daemon-reload"])?;
    }
    Ok(())
}

/// A one-line-per-fact status: the unit, and the daemon's `/v1/info` when it answers.
pub fn status(listen: &Listen) -> AppResult<String> {
    let mut out = String::new();
    let path = unit_path()?;
    out.push_str(&format!("unit:     {} ({})\n", path.display(), if path.exists() { "installed" } else { "not installed" }));
    let state = |verb: &str| {
        systemctl(&[verb, UNIT_NAME]).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_else(|_| "unknown".into())
    };
    out.push_str(&format!("enabled:  {}\nactive:   {}\n", state("is-enabled"), state("is-active")));
    out.push_str(&match probe_info(listen) {
        Ok(v) => format!(
            "daemon:   answering on {listen}: protocol {}, cr {}, host {}\n",
            v["protocol"], v["version"].as_str().unwrap_or("?"), v["host"].as_str().unwrap_or("?")
        ),
        Err(e) => format!("daemon:   not answering on {listen} ({e})\n"),
    });
    Ok(out)
}

/// `GET /v1/info` over a blocking socket.
pub fn probe_info(listen: &Listen) -> Result<serde_json::Value, String> {
    let request = "GET /v1/info HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nAccept: application/json\r\n\r\n";
    let mut raw = Vec::new();
    match listen {
        #[cfg(unix)]
        Listen::Unix(p) => exchange(std::os::unix::net::UnixStream::connect(p).map_err(|e| e.to_string())?, request, &mut raw)?,
        #[cfg(not(unix))]
        Listen::Unix(_) => return Err("unix sockets are not supported on this platform".to_string()),
        Listen::Tcp(a) => exchange(std::net::TcpStream::connect_timeout(a, std::time::Duration::from_secs(2)).map_err(|e| e.to_string())?, request, &mut raw)?,
    }
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text.split_once("\r\n\r\n").ok_or("malformed response")?;
    if !head.starts_with("HTTP/1.1 200") {
        return Err(head.lines().next().unwrap_or("bad response").to_string());
    }
    serde_json::from_str(body.trim()).map_err(|e| e.to_string())
}

fn exchange<S: Read + Write>(mut s: S, request: &str, out: &mut Vec<u8>) -> Result<(), String> {
    s.write_all(request.as_bytes()).map_err(|e| e.to_string())?;
    s.read_to_end(out).map_err(|e| e.to_string())?;
    Ok(())
}

/// Absolute form of a user-given path, for baking into a unit.
pub fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> UnitSpec {
        UnitSpec {
            exe: "/home/u/.local/bin/cr".into(),
            listen: Listen::Unix("/run/user/1000/codereview/worker.sock".into()),
            app_db: None,
            worker_db: None,
            max_concurrency: None,
            log_level: None,
        }
    }

    #[test]
    fn unit_keeps_runs_alive_across_a_restart() {
        let t = unit_text(&spec());
        assert!(t.contains("ExecStart=/home/u/.local/bin/cr worker --listen unix:/run/user/1000/codereview/worker.sock\n"), "{t}");
        assert!(t.contains("KillMode=process"));
        assert!(t.contains("WantedBy=default.target"));
        assert!(t.contains("Restart=on-failure"));
    }

    #[test]
    fn unit_bakes_in_the_flags_and_quotes_spaces() {
        let s = UnitSpec {
            app_db: Some("/data/my dir/app.db".into()),
            worker_db: Some("/data/worker.db".into()),
            max_concurrency: Some(2),
            log_level: Some("debug".into()),
            ..spec()
        };
        let t = unit_text(&s);
        let line = t.lines().find(|l| l.starts_with("ExecStart=")).unwrap();
        assert_eq!(
            line,
            "ExecStart=/home/u/.local/bin/cr --db \"/data/my dir/app.db\" worker --listen unix:/run/user/1000/codereview/worker.sock --worker-db /data/worker.db --max-concurrency 2 --log-level debug"
        );
    }
}
