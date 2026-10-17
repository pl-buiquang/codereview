//! Tauri commands for the sync engine (`crate::sync`).

use tauri::{AppHandle, Emitter, State};

use crate::db::Db;
use crate::error::AppResult;
use crate::mirror;
use crate::sync::scheduler::{SyncConfig, SyncStatus};
use crate::sync::{Mode, MirrorUpdated, Scope, SyncEngine, SyncReport, SyncRequest, MIRROR_UPDATED_EVENT};

#[tauri::command]
pub fn sync_hint(scope: Scope, active: bool, visible_ids: Option<Vec<String>>, engine: State<SyncEngine>) {
    engine.hint(scope, active, visible_ids.unwrap_or_default());
}

#[tauri::command]
pub fn sync_window_state(visible: bool, focused: bool, engine: State<SyncEngine>) {
    engine.window(visible, focused);
}

#[tauri::command]
pub fn sync_configure(config: SyncConfig, engine: State<SyncEngine>) {
    engine.configure(config);
}

#[tauri::command]
pub fn sync_status(engine: State<SyncEngine>) -> SyncStatus {
    engine.status()
}

/// `async` so the sync runs off the UI thread.
#[tauri::command]
pub async fn sync_now(
    scope: Scope,
    mode: Mode,
    visible_ids: Option<Vec<String>>,
    engine: State<'_, SyncEngine>,
) -> AppResult<SyncReport> {
    engine.run_now(SyncRequest { visible_ids: visible_ids.unwrap_or_default(), ..SyncRequest::new(scope, mode) })
}

/// Wipe the mirror, then let a light sync rebuild what the views reference.
#[tauri::command]
pub fn reset_mirror(app: AppHandle, db: State<Db>, engine: State<SyncEngine>) -> AppResult<()> {
    mirror::store::reset(&db.0.lock().unwrap())?;
    log::debug!(target: "mirror", "reset_mirror: emit {MIRROR_UPDATED_EVENT}");
    let _ = app.emit(MIRROR_UPDATED_EVENT, MirrorUpdated { scopes: vec![Scope::Inbox, Scope::Board], ids: vec![] });
    engine.spawn(SyncRequest::new(Scope::All, Mode::Light));
    Ok(())
}
