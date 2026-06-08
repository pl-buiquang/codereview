mod anchor;
mod commands;
mod db;
mod error;
mod export;
mod gh;
mod git;
mod inbox;
mod path_env;
mod tools;

use std::sync::Mutex;

use tauri::Manager;

use db::Db;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    path_env::ensure_login_path();
    tools::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            let db_path = match std::env::var_os("CODEREVIEW_DB") {
                Some(p) => std::path::PathBuf::from(p),
                None => {
                    let dir = app.path().app_data_dir()?;
                    std::fs::create_dir_all(&dir)?;
                    dir.join("codereview.db")
                }
            };
            if let Some(parent) = db_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let conn = db::open(&db_path)?;
            app.manage(Db(Mutex::new(conn)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::repo::add_repository,
            commands::repo::list_repositories,
            commands::repo::remove_repository,
            commands::repo::list_branches,
            commands::repo::diff_refs,
            commands::review::create_review,
            commands::review::list_reviews,
            commands::review::get_review,
            commands::review::set_file_viewed,
            commands::review::update_review,
            commands::review::delete_review,
            commands::review::add_comment,
            commands::review::add_file_comment,
            commands::review::add_file_view_comment,
            commands::review::update_comment,
            commands::review::delete_comment,
            commands::review::create_review_for_pr,
            commands::review::review_diff,
            commands::review::file_source,
            commands::review::publish_review,
            commands::review::refresh_review,
            commands::review::reanchor_comments,
            commands::export::preview_review,
            commands::export::export_review,
            commands::editor::open_in_default_app,
            commands::editor::open_url,
            commands::gh::gh_auth_status,
            commands::gh::list_prs,
            commands::gh::pr_meta,
            commands::gh::pr_review_threads,
            commands::gh::check_environment,
            commands::inbox::refresh_inbox,
            commands::inbox::list_inbox,
            commands::inbox::list_archive,
            commands::inbox::list_closed,
            commands::inbox::engage_item,
            commands::inbox::unengage_item,
            commands::inbox::untrack_item,
            commands::inbox::retrack_item,
            commands::inbox::open_pr_review,
            commands::inbox::inbox_meta,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
