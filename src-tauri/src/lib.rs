mod commands;
mod db;
mod error;
mod export;
mod gh;
mod git;
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
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let conn = db::open(&dir.join("codereview.db"))?;
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
            commands::review::update_comment,
            commands::review::delete_comment,
            commands::review::create_review_for_pr,
            commands::review::review_diff,
            commands::review::publish_review,
            commands::export::preview_review,
            commands::export::export_review,
            commands::gh::gh_auth_status,
            commands::gh::list_prs,
            commands::gh::check_environment,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
