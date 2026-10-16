use tauri::State;

use crate::db::Db;
use crate::error::AppResult;
use crate::prompts::{self, PromptInfo};

#[tauri::command]
pub fn prompts_list(db: State<Db>) -> AppResult<Vec<PromptInfo>> {
    let conn = db.0.lock().unwrap();
    prompts::list(&conn)
}

#[tauri::command]
pub fn prompt_set(key: String, body: String, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    prompts::set(&conn, &key, &body)
}

#[tauri::command]
pub fn prompt_reset(key: String, db: State<Db>) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    prompts::reset(&conn, &key)
}
