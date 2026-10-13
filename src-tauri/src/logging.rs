//! Minimal file-backed logger for the Settings → Logs viewer. A handful of
//! existing best-effort warning sites call `logging::warn` instead of a bare
//! `eprintln!`, so the log file has useful content without rewiring every
//! print site in the app. The `cr` CLI never installs this logger, so
//! `log::warn!` is a no-op there and its own `eprintln!`-based terminal
//! output is unaffected.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;

/// Log files roll over to `.old` past this size rather than growing unbounded.
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// The directory the active log file lives in, managed as Tauri state
/// alongside `db::DbPath` (same directory as the database).
pub struct LogDir(pub PathBuf);

struct FileLogger {
    file: Mutex<File>,
}

impl log::Log for FileLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        let Ok(mut file) = self.file.lock() else { return };
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
    if log::set_boxed_logger(Box::new(FileLogger { file: Mutex::new(file) })).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
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
