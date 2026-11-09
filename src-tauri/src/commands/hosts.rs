//! Tauri commands for remote hosts (Settings → AI → Hosts).

use tauri::State;

use crate::db::Db;
use crate::error::AppResult;
use crate::remote::host::{self, Host, HostInput};

#[tauri::command]
pub fn host_list(db: State<Db>) -> AppResult<Vec<Host>> {
    host::list(&db.0.lock().unwrap())
}

#[tauri::command]
pub fn host_save(host: HostInput, db: State<Db>) -> AppResult<Host> {
    host::save(&db.0.lock().unwrap(), host)
}

#[tauri::command]
pub fn host_delete(id: i64, db: State<Db>) -> AppResult<()> {
    host::delete(&db.0.lock().unwrap(), id)
}
