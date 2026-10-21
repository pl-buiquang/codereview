//! Global switch for every app-spawned `claude -p` feature (chat, focus, AI review, board
//! overview, session summaries, routines). The frontend owns the setting (Settings → AI, off by
//! default) and pushes it here on startup and on change; the backend refuses to start AI work
//! while it is off, so nothing slips through a stale UI.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::routines::manager::RoutineManager;

static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn ensure_enabled() -> AppResult<()> {
    if is_enabled() {
        Ok(())
    } else {
        Err(AppError::Other("AI features are turned off (Settings → AI)".into()))
    }
}

#[tauri::command]
pub fn set_ai_enabled(enabled: bool, app: AppHandle, manager: State<RoutineManager>) {
    ENABLED.store(enabled, Ordering::Relaxed);
    // Runs queued while AI was off start now.
    if enabled {
        manager.pump(&app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_enabled_follows_the_flag() {
        ENABLED.store(false, Ordering::Relaxed);
        assert!(ensure_enabled().is_err());
        ENABLED.store(true, Ordering::Relaxed);
        assert!(ensure_enabled().is_ok());
        ENABLED.store(false, Ordering::Relaxed);
    }
}
