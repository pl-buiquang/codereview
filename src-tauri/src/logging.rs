//! Minimal file-backed logger for the Settings → Logs viewer. A handful of
//! existing best-effort warning sites call `logging::warn` instead of a bare
//! `eprintln!`, so the log file has useful content without rewiring every
//! print site in the app. The `cr` CLI never installs this logger, so
//! `log::warn!` is a no-op there and its own `eprintln!`-based terminal
//! output is unaffected.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use chrono::Utc;

/// Log files roll over to `.old` past this size rather than growing unbounded.
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// The directory the active log file lives in, managed as Tauri state
/// alongside `db::DbPath` (same directory as the database).
pub struct LogDir(pub PathBuf);

/// Level used when the user never picked one (Settings → Logs).
pub const DEFAULT_LEVEL: log::LevelFilter = log::LevelFilter::Warn;

/// Name of the small file (next to the log) that remembers the chosen level,
/// so lines emitted during startup already honor it.
const LEVEL_FILE: &str = "log-level";

/// Re-check the file size every this many writes, so a Debug session can't
/// grow the log far past `MAX_LOG_BYTES` between launches.
const ROLL_CHECK_EVERY: u32 = 500;

struct FileLogger {
    file: Mutex<File>,
    path: PathBuf,
    writes: AtomicU32,
}

impl FileLogger {
    /// Roll the log to `.old` once it's oversized and reopen a fresh file.
    fn roll_if_needed(&self, file: &mut File) {
        if file.metadata().map(|m| m.len()).unwrap_or(0) <= MAX_LOG_BYTES {
            return;
        }
        let _ = std::fs::rename(&self.path, self.path.with_extension("log.old"));
        if let Ok(fresh) = OpenOptions::new().create(true).append(true).open(&self.path) {
            *file = fresh;
        }
    }
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let Ok(mut file) = self.file.lock() else { return };
        if self.writes.fetch_add(1, Ordering::Relaxed) % ROLL_CHECK_EVERY == ROLL_CHECK_EVERY - 1 {
            self.roll_if_needed(&mut file);
        }
        let _ = writeln!(
            file,
            "{} [{}] {}: {}",
            Utc::now().to_rfc3339(),
            record.level(),
            record.target(),
            record.args()
        );
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
    }
}

pub fn log_file_path(dir: &Path) -> PathBuf {
    dir.join(crate::db_path::log_filename())
}

/// Parse a level name from the frontend ("error" | "warn" | "info" | "debug").
pub fn parse_level(name: &str) -> Option<log::LevelFilter> {
    match name.trim().to_ascii_lowercase().as_str() {
        "error" => Some(log::LevelFilter::Error),
        "warn" => Some(log::LevelFilter::Warn),
        "info" => Some(log::LevelFilter::Info),
        "debug" => Some(log::LevelFilter::Debug),
        _ => None,
    }
}

pub fn level_name(level: log::LevelFilter) -> &'static str {
    match level {
        log::LevelFilter::Off | log::LevelFilter::Error => "error",
        log::LevelFilter::Warn => "warn",
        log::LevelFilter::Info => "info",
        log::LevelFilter::Debug | log::LevelFilter::Trace => "debug",
    }
}

fn level_file(dir: &Path) -> PathBuf {
    dir.join(LEVEL_FILE)
}

fn read_saved_level(dir: &Path) -> log::LevelFilter {
    std::fs::read_to_string(level_file(dir))
        .ok()
        .and_then(|s| parse_level(&s))
        .unwrap_or(DEFAULT_LEVEL)
}

/// Apply `level` to the running logger and remember it for the next launch.
pub fn set_level(dir: &Path, level: log::LevelFilter) {
    log::set_max_level(level);
    let _ = std::fs::write(level_file(dir), level_name(level));
    // Always visible (it's the Error..Debug filter itself that we just changed).
    eprintln!("[logging] level set to {}", level_name(level));
}

pub fn current_level() -> &'static str {
    level_name(log::max_level())
}

/// Record a line sent by the frontend under a `ui:`-prefixed target.
pub fn frontend(level: &str, target: &str, message: &str) {
    let lvl = match level {
        "error" => log::Level::Error,
        "warn" => log::Level::Warn,
        "info" => log::Level::Info,
        _ => log::Level::Debug,
    };
    log::log!(target: &format!("ui:{target}"), lvl, "{message}");
}

/// Install the global logger, writing to `log_file_path(dir)`. Rolls the
/// previous file to `<name>.old` once it exceeds `MAX_LOG_BYTES`. A second
/// call anywhere in the process (there should only ever be one) is a no-op.
pub fn init(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = log_file_path(dir);
    if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > MAX_LOG_BYTES {
        let _ = std::fs::rename(&path, path.with_extension("log.old"));
    }
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    let logger = FileLogger { file: Mutex::new(file), path, writes: AtomicU32::new(0) };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(read_saved_level(dir));
    }
    Ok(())
}

/// Print to stderr (dev-terminal visibility, matching the app's existing
/// `eprintln!` convention) and mirror the same message to the log file via
/// `log::warn!`.
pub fn warn(msg: &str) {
    eprintln!("{msg}");
    log::warn!("{msg}");
}

/// The last `max_lines` lines of the log file, oldest first. Empty string if
/// the file doesn't exist yet (logger never installed, or nothing logged).
pub fn read_tail(dir: &Path, max_lines: usize) -> std::io::Result<String> {
    let path = log_file_path(dir);
    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            let lines: Vec<&str> = contents.lines().collect();
            let start = lines.len().saturating_sub(max_lines);
            Ok(lines[start..].join("\n"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn read_tail_truncates_to_the_last_n_lines() {
        let dir = TempDir::new().unwrap();
        std::fs::write(log_file_path(dir.path()), "a\nb\nc\nd\n").unwrap();
        assert_eq!(read_tail(dir.path(), 2).unwrap(), "c\nd");
        assert_eq!(read_tail(dir.path(), 10).unwrap(), "a\nb\nc\nd");
    }

    #[test]
    fn parses_level_names() {
        assert_eq!(parse_level("warn"), Some(log::LevelFilter::Warn));
        assert_eq!(parse_level(" Debug\n"), Some(log::LevelFilter::Debug));
        assert_eq!(parse_level("loud"), None);
        assert_eq!(level_name(log::LevelFilter::Info), "info");
    }

    #[test]
    fn saved_level_defaults_to_warn_and_round_trips() {
        let dir = TempDir::new().unwrap();
        assert_eq!(read_saved_level(dir.path()), log::LevelFilter::Warn);
        std::fs::write(level_file(dir.path()), "debug").unwrap();
        assert_eq!(read_saved_level(dir.path()), log::LevelFilter::Debug);
    }

    #[test]
    fn read_tail_is_empty_when_no_log_file_exists() {
        let dir = TempDir::new().unwrap();
        assert_eq!(read_tail(dir.path(), 10).unwrap(), "");
    }

    #[test]
    fn init_rolls_an_oversized_log_file() {
        let dir = TempDir::new().unwrap();
        let path = log_file_path(dir.path());
        std::fs::write(&path, "x".repeat((MAX_LOG_BYTES + 1) as usize)).unwrap();
        init(dir.path()).unwrap();
        assert!(path.with_extension("log.old").exists());
        // The logger starts a fresh, small file rather than appending to the huge one.
        assert!(std::fs::metadata(&path).unwrap().len() < MAX_LOG_BYTES);
    }
}
