//! Hosts: the machines running `cr serve` that profiles can point at. A host is an SSH alias plus
//! the daemon's endpoint on that machine; nothing secret is stored.

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// The only host type implemented. `direct` (url + token) is reserved in the schema.
pub const TYPE_SSH: &str = "ssh";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Host {
    pub id: i64,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    /// Alias from `~/.ssh/config` (or `user@host`).
    pub ssh_target: String,
    /// Absolute socket path, or a loopback port, of the daemon on the remote machine.
    pub remote_endpoint: String,
    pub enabled: bool,
    pub last_seen: Option<String>,
    pub last_protocol: Option<i64>,
    pub last_version: Option<String>,
}

impl Host {
    fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
            kind: row.get("type")?,
            ssh_target: row.get("ssh_target")?,
            remote_endpoint: row.get("remote_endpoint")?,
            enabled: row.get::<_, i64>("enabled")? != 0,
            last_seen: row.get("last_seen")?,
            last_protocol: row.get("last_protocol")?,
            last_version: row.get("last_version")?,
        })
    }
}

/// What the Hosts editor sends. `id: None` creates a host.
#[derive(Debug, Deserialize)]
pub struct HostInput {
    pub id: Option<i64>,
    pub name: String,
    pub ssh_target: String,
    pub remote_endpoint: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

/// Where the daemon listens on the remote machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteEndpoint {
    Socket(String),
    Port(u16),
}

/// Parse a stored endpoint: an absolute socket path or a port number.
pub fn parse_endpoint(s: &str) -> AppResult<RemoteEndpoint> {
    let s = s.trim();
    if s.is_empty() {
        return Err(AppError::Other("the remote endpoint is empty: give the daemon's socket path (absolute) or its loopback port".into()));
    }
    if s.chars().all(|c| c.is_ascii_digit()) {
        return match s.parse::<u16>() {
            Ok(p) if p > 0 => Ok(RemoteEndpoint::Port(p)),
            _ => Err(AppError::Other(format!("`{s}` is not a valid port"))),
        };
    }
    if !s.starts_with('/') {
        return Err(AppError::Other(format!(
            "`{s}`: the remote socket path must be absolute (ssh does not expand `~` or `$XDG_RUNTIME_DIR`); run `echo $XDG_RUNTIME_DIR/codereview/serve.sock` on the host"
        )));
    }
    // `:` separates the fields of an ssh forward spec, so it cannot appear in a path.
    if s.contains(':') || s.chars().any(char::is_control) {
        return Err(AppError::Other(format!("`{s}`: the socket path may not contain `:` or control characters")));
    }
    Ok(RemoteEndpoint::Socket(s.to_string()))
}

/// An ssh target is an alias or `user@host`, never something `ssh` could read as an option.
pub fn validate_ssh_target(s: &str) -> AppResult<()> {
    let s = s.trim();
    if s.is_empty() {
        return Err(AppError::Other("the SSH target is empty: use an alias from ~/.ssh/config".into()));
    }
    if s.starts_with('-') || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(AppError::Other(format!("`{s}` is not a valid SSH target (no spaces, no leading `-`)")));
    }
    Ok(())
}

pub fn list(conn: &Connection) -> AppResult<Vec<Host>> {
    let mut stmt = conn.prepare("SELECT * FROM host ORDER BY name COLLATE NOCASE, id")?;
    let rows = stmt.query_map([], Host::from_row)?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

pub fn get(conn: &Connection, id: i64) -> AppResult<Option<Host>> {
    Ok(conn.query_row("SELECT * FROM host WHERE id = ?1", params![id], Host::from_row).optional()?)
}

/// The host `id`, or a readable error.
pub fn require(conn: &Connection, id: i64) -> AppResult<Host> {
    get(conn, id)?.ok_or_else(|| AppError::Other(format!("host {id} not found (was it removed?)")))
}

pub fn save(conn: &Connection, input: HostInput) -> AppResult<Host> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Other("a host needs a name".into()));
    }
    validate_ssh_target(&input.ssh_target)?;
    parse_endpoint(&input.remote_endpoint)?;
    let (target, endpoint) = (input.ssh_target.trim().to_string(), input.remote_endpoint.trim().to_string());
    let now = chrono::Utc::now().to_rfc3339();
    let taken = |e: rusqlite::Error| match e {
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
            AppError::Other(format!("a host named “{name}” already exists"))
        }
        other => other.into(),
    };
    let id = match input.id {
        Some(id) => {
            // A run in flight is followed by its remote id on this host's daemon: moving the host would
            // make the mirror ask another daemon about it.
            if let Some(old) = get(conn, id)? {
                if (old.ssh_target != target || old.remote_endpoint != endpoint) && active_runs(conn, id)? > 0 {
                    return Err(AppError::Other(format!(
                        "“{}” still has active runs: wait for them (or stop them) before changing its SSH target or endpoint",
                        old.name
                    )));
                }
            }
            // The connection facts of a host that changed address are stale.
            let changed = conn
                .execute(
                    "UPDATE host SET name = ?2, ssh_target = ?3, remote_endpoint = ?4, enabled = ?5, updated_at = ?6,
                            last_seen = CASE WHEN ssh_target = ?3 AND remote_endpoint = ?4 THEN last_seen END,
                            last_protocol = CASE WHEN ssh_target = ?3 AND remote_endpoint = ?4 THEN last_protocol END,
                            last_version = CASE WHEN ssh_target = ?3 AND remote_endpoint = ?4 THEN last_version END
                     WHERE id = ?1",
                    params![id, name, target, endpoint, input.enabled, now],
                )
                .map_err(taken)?;
            if changed == 0 {
                return Err(AppError::Other(format!("host {id} not found")));
            }
            id
        }
        None => {
            conn.execute(
                "INSERT INTO host (name, type, ssh_target, remote_endpoint, enabled, created_at, updated_at)
                 VALUES (?1, 'ssh', ?2, ?3, ?4, ?5, ?5)",
                params![name, target, endpoint, input.enabled, now],
            )
            .map_err(taken)?;
            conn.last_insert_rowid()
        }
    };
    require(conn, id)
}

fn active_runs(conn: &Connection, host_id: i64) -> AppResult<i64> {
    Ok(conn.query_row("SELECT COUNT(*) FROM routine_run WHERE host_id = ?1 AND status IN ('queued', 'running')", params![host_id], |r| r.get(0))?)
}

/// Remove a host. Refused while a profile points at it or a run on it is still going: both would
/// otherwise silently change meaning.
pub fn delete(conn: &Connection, id: i64) -> AppResult<()> {
    let host = require(conn, id)?;
    let profiles: Vec<String> = conn
        .prepare("SELECT name FROM ai_profile WHERE host_id = ?1 ORDER BY name")?
        .query_map(params![id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if !profiles.is_empty() {
        return Err(AppError::Other(format!(
            "“{}” is used by the profile{} {}: delete or move {} first",
            host.name,
            if profiles.len() > 1 { "s" } else { "" },
            profiles.iter().map(|p| format!("“{p}”")).collect::<Vec<_>>().join(", "),
            if profiles.len() > 1 { "them" } else { "it" },
        )));
    }
    let active: i64 = conn.query_row(
        "SELECT COUNT(*) FROM routine_run WHERE host_id = ?1 AND status IN ('queued', 'running')",
        params![id],
        |r| r.get(0),
    )?;
    if active > 0 {
        return Err(AppError::Other(format!("“{}” still has {active} active run(s): stop them first", host.name)));
    }
    conn.execute("DELETE FROM host WHERE id = ?1", params![id])?;
    Ok(())
}

/// Remember what a successful `/v1/info` told us.
pub fn record_seen(conn: &Connection, id: i64, protocol: i64, version: &str) {
    let _ = conn.execute(
        "UPDATE host SET last_seen = ?2, last_protocol = ?3, last_version = ?4 WHERE id = ?1",
        params![id, chrono::Utc::now().to_rfc3339(), protocol, version],
    );
}

/// Remember the protocol and version of a host that answered but cannot be used.
pub fn record_mismatch(conn: &Connection, id: i64, protocol: i64, version: &str) {
    let _ = conn.execute(
        "UPDATE host SET last_protocol = ?2, last_version = ?3 WHERE id = ?1",
        params![id, protocol, version],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;

    fn input(name: &str) -> HostInput {
        HostInput { id: None, name: name.into(), ssh_target: "koza".into(), remote_endpoint: "/run/user/1000/codereview/serve.sock".into(), enabled: true }
    }

    #[test]
    fn hosts_round_trip() {
        let conn = open_memory();
        let h = save(&conn, input("koza")).unwrap();
        assert_eq!((h.kind.as_str(), h.enabled, h.last_seen.clone()), ("ssh", true, None));
        let mut edit = input("koza.tv");
        edit.id = Some(h.id);
        edit.enabled = false;
        let h = save(&conn, edit).unwrap();
        assert_eq!((h.name.as_str(), h.enabled), ("koza.tv", false));
        assert_eq!(list(&conn).unwrap().len(), 1);
        delete(&conn, h.id).unwrap();
        assert!(list(&conn).unwrap().is_empty());
        assert!(require(&conn, h.id).unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn names_are_unique_case_insensitively_and_input_is_validated() {
        let conn = open_memory();
        save(&conn, input("Koza")).unwrap();
        assert!(save(&conn, input("koza")).unwrap_err().to_string().contains("already exists"));
        assert!(save(&conn, input("  ")).is_err());
        let mut bad = input("x");
        bad.ssh_target = "-oProxyCommand=evil".into();
        assert!(save(&conn, bad).is_err());
        let mut spaced = input("x");
        spaced.ssh_target = "a b".into();
        assert!(save(&conn, spaced).is_err());
        let mut rel = input("x");
        rel.remote_endpoint = "~/serve.sock".into();
        assert!(save(&conn, rel).unwrap_err().to_string().contains("absolute"));
        let mut missing = input("x");
        missing.id = Some(99);
        assert!(save(&conn, missing).is_err());
    }

    #[test]
    fn endpoints_are_a_socket_path_or_a_port() {
        assert_eq!(parse_endpoint(" 7777 ").unwrap(), RemoteEndpoint::Port(7777));
        assert_eq!(parse_endpoint("/run/x.sock").unwrap(), RemoteEndpoint::Socket("/run/x.sock".into()));
        for bad in ["", "0", "70000", "rel.sock", "/a:b", "$XDG_RUNTIME_DIR/x"] {
            assert!(parse_endpoint(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn changing_the_address_forgets_what_the_old_one_reported() {
        let conn = open_memory();
        let h = save(&conn, input("a")).unwrap();
        record_seen(&conn, h.id, 1, "0.6.0");
        let seen = get(&conn, h.id).unwrap().unwrap();
        assert_eq!((seen.last_protocol, seen.last_version.as_deref()), (Some(1), Some("0.6.0")));
        assert!(seen.last_seen.is_some());
        // a rename keeps it, a new endpoint clears it
        let mut e = input("b");
        e.id = Some(h.id);
        assert_eq!(save(&conn, e).unwrap().last_protocol, Some(1));
        let mut e = input("b");
        e.id = Some(h.id);
        e.remote_endpoint = "7777".into();
        assert_eq!(save(&conn, e).unwrap().last_protocol, None);
    }

    #[test]
    fn a_host_with_active_runs_cannot_be_repointed_but_can_be_renamed() {
        let conn = open_memory();
        let h = save(&conn, input("a")).unwrap();
        conn.execute(
            "INSERT INTO routine (key, name, context_type, prompt_template, created_at, updated_at) VALUES ('k', 'k', 'pr', 'p', 'n', 'n')",
            [],
        )
        .unwrap();
        let rid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO routine_run (routine_id, context_type, context_key, context_json, status, queued_at, host_id)
             VALUES (?1, 'pr', 'a', '{}', 'running', 'n', ?2)",
            params![rid, h.id],
        )
        .unwrap();
        let mut moved = input("a");
        moved.id = Some(h.id);
        moved.remote_endpoint = "7777".into();
        assert!(save(&conn, moved).unwrap_err().to_string().contains("still has active runs"));
        let mut other_target = input("a");
        other_target.id = Some(h.id);
        other_target.ssh_target = "elsewhere".into();
        assert!(save(&conn, other_target).is_err());
        // a rename or disabling keeps the address and is fine
        let mut renamed = input("b");
        renamed.id = Some(h.id);
        renamed.enabled = false;
        assert_eq!(save(&conn, renamed).unwrap().name, "b");
        conn.execute("UPDATE routine_run SET status = 'ok'", []).unwrap();
        let mut moved = input("b");
        moved.id = Some(h.id);
        moved.remote_endpoint = "7777".into();
        assert_eq!(save(&conn, moved).unwrap().remote_endpoint, "7777");
    }

    #[test]
    fn a_host_in_use_cannot_be_removed() {
        let conn = open_memory();
        let h = save(&conn, input("a")).unwrap();
        conn.execute(
            "INSERT INTO ai_profile (name, harness, host_id, created_at, updated_at) VALUES ('Remote', 'claude', ?1, 'n', 'n')",
            params![h.id],
        )
        .unwrap();
        assert!(delete(&conn, h.id).unwrap_err().to_string().contains("“Remote”"));
        conn.execute("DELETE FROM ai_profile WHERE host_id = ?1", params![h.id]).unwrap();
        conn.execute(
            "INSERT INTO routine (key, name, context_type, prompt_template, created_at, updated_at) VALUES ('k', 'k', 'pr', 'p', 'n', 'n')",
            [],
        )
        .unwrap();
        let rid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO routine_run (routine_id, context_type, context_key, context_json, status, queued_at, host_id)
             VALUES (?1, 'pr', 'a', '{}', 'running', 'n', ?2)",
            params![rid, h.id],
        )
        .unwrap();
        assert!(delete(&conn, h.id).unwrap_err().to_string().contains("active run"));
        conn.execute("UPDATE routine_run SET status = 'ok'", []).unwrap();
        delete(&conn, h.id).unwrap();
    }

    #[test]
    fn migration_adds_the_remote_columns_with_local_defaults() {
        let conn = open_memory();
        crate::routines::seed(&conn).unwrap();
        let rid = crate::routines::list(&conn).unwrap()[0].id;
        conn.execute(
            "INSERT INTO routine_run (routine_id, context_type, context_key, context_json, status, queued_at)
             VALUES (?1, 'pr', 'a', '{}', 'queued', 'n')",
            params![rid],
        )
        .unwrap();
        let (host, remote, seq): (Option<i64>, Option<i64>, i64) = conn
            .query_row("SELECT host_id, remote_run_id, last_remote_seq FROM routine_run", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap();
        assert_eq!((host, remote, seq), (None, None, 0));
        let table_has = |table: &str, col: &str| {
            conn.prepare(&format!("PRAGMA table_info({table})")).unwrap().query_map([], |r| r.get::<_, String>(1)).unwrap().any(|c| c.unwrap() == col)
        };
        assert!(table_has("ai_run", "host_id"));
        assert!(table_has("remote_applied_op", "op_id"));
    }
}
