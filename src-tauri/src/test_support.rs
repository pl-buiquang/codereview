//! Helpers for tests that snapshot the command lines the app builds for AI CLIs.

use std::process::Command;

/// Arguments of `cmd`, program excluded (it depends on the machine's PATH).
pub fn argv(cmd: &Command) -> Vec<String> {
    cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect()
}

/// Environment overrides of `cmd`, sorted, `PATH` excluded (it depends on the login shell).
pub fn envs(cmd: &Command) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = cmd
        .get_envs()
        .filter(|(k, _)| *k != "PATH")
        .map(|(k, v)| (k.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned()).unwrap_or_default()))
        .collect();
    v.sort();
    v
}

pub fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}
