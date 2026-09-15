use std::path::PathBuf;

pub fn resolve_db_path(cli_override: Option<&str>) -> PathBuf {
    if let Some(p) = cli_override {
        return PathBuf::from(p);
    }
    if let Some(p) = std::env::var_os("CODEREVIEW_DB") {
        return PathBuf::from(p);
    }
    default_db_path()
}

#[cfg(target_os = "macos")]
fn default_db_path() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME not set");
    PathBuf::from(home).join("Library/Application Support/com.codereview.app/codereview.db")
}

#[cfg(target_os = "linux")]
fn default_db_path() -> PathBuf {
    let data = std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").expect("HOME not set");
        format!("{home}/.local/share")
    });
    PathBuf::from(data).join("com.codereview.app/codereview.db")
}

#[cfg(target_os = "windows")]
fn default_db_path() -> PathBuf {
    let appdata = std::env::var("APPDATA").expect("APPDATA not set");
    PathBuf::from(appdata).join("com.codereview.app/codereview.db")
}
