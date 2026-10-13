use std::path::PathBuf;

/// `codereview-dev.db` for a debug build (`cargo run` / `pnpm tauri dev`),
/// `codereview.db` for a release build (`pnpm tauri build`) — so a local dev
/// session never reads or writes the real installed app's database. Shared
/// by the `cr` CLI (via `resolve_db_path`) and the main app (`lib.rs::run`),
/// which both resolve to the same directory, so this must be the only place
/// that decides the filename.
pub fn db_filename() -> &'static str {
    if cfg!(debug_assertions) {
        "codereview-dev.db"
    } else {
        "codereview.db"
    }
}

/// Same dev/release split as `db_filename`, for the log file.
pub fn log_filename() -> &'static str {
    if cfg!(debug_assertions) {
        "codereview-dev.log"
    } else {
        "codereview.log"
    }
}

pub fn resolve_db_path(cli_override: Option<&str>) -> PathBuf {
    if let Some(p) = cli_override {
        return PathBuf::from(p);
    }
    if let Some(p) = std::env::var_os("CODEREVIEW_DB") {
        return PathBuf::from(p);
    }
    app_data_dir().join(db_filename())
}

#[cfg(target_os = "macos")]
pub fn app_data_dir() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME not set");
    PathBuf::from(home).join("Library/Application Support/com.codereview.app")
}

#[cfg(target_os = "linux")]
pub fn app_data_dir() -> PathBuf {
    let data = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").expect("HOME not set");
        format!("{home}/.local/share")
    });
    PathBuf::from(data).join("com.codereview.app")
}

#[cfg(target_os = "windows")]
pub fn app_data_dir() -> PathBuf {
    let appdata = std::env::var("APPDATA").expect("APPDATA not set");
    PathBuf::from(appdata).join("com.codereview.app")
}
