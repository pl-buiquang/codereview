mod anchor;
pub mod commands;
pub mod db;
pub mod db_path;
pub mod error;
pub mod export;
pub mod gh;
pub mod git;
pub mod inbox;
mod path_env;
pub mod provider;
pub mod tools;

use std::sync::Mutex;

use tauri::{Emitter, Manager};

use db::Db;

const CLOSE_TAB_MENU_ID: &str = "close_tab";
const CLOSE_ACTIVE_TAB_EVENT: &str = "close-active-tab";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    path_env::ensure_login_path();
    tools::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .menu(app_menu)
        .on_menu_event(|app, event| {
            if event.id() == CLOSE_TAB_MENU_ID {
                let _ = app.emit(CLOSE_ACTIVE_TAB_EVENT, ());
            }
        })
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
            commands::review::set_comment_resolved,
            commands::review::create_review_for_pr,
            commands::review::review_diff,
            commands::review::file_source,
            commands::review::publish_review,
            commands::review::publish_review_pending,
            commands::review::submit_pending_review,
            commands::review::discard_pending_review,
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
            commands::gh::reply_to_thread,
            commands::gh::set_pr_thread_resolved,
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

fn app_menu<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> tauri::Result<tauri::menu::Menu<R>> {
    use tauri::menu::{
        AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu, HELP_SUBMENU_ID,
        WINDOW_SUBMENU_ID,
    };

    let pkg_info = app.package_info();
    let config = app.config();
    let about_metadata = AboutMetadata {
        name: Some(pkg_info.name.clone()),
        version: Some(pkg_info.version.to_string()),
        copyright: config.bundle.copyright.clone(),
        authors: config.bundle.publisher.clone().map(|p| vec![p]),
        ..Default::default()
    };

    let window_menu = Submenu::with_id_and_items(
        app,
        WINDOW_SUBMENU_ID,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
        ],
    )?;

    let help_menu = Submenu::with_id_and_items(
        app,
        HELP_SUBMENU_ID,
        "Help",
        true,
        &[
            #[cfg(not(target_os = "macos"))]
            &PredefinedMenuItem::about(app, None, Some(about_metadata.clone()))?,
        ],
    )?;

    Menu::with_items(
        app,
        &[
            #[cfg(target_os = "macos")]
            &Submenu::with_items(
                app,
                pkg_info.name.clone(),
                true,
                &[
                    &PredefinedMenuItem::about(app, None, Some(about_metadata))?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::services(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::hide(app, None)?,
                    &PredefinedMenuItem::hide_others(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::quit(app, None)?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "File",
                true,
                &[
                    &MenuItem::with_id(
                        app,
                        CLOSE_TAB_MENU_ID,
                        "Close Tab",
                        true,
                        Some("CmdOrCtrl+W"),
                    )?,
                    #[cfg(not(target_os = "macos"))]
                    &PredefinedMenuItem::separator(app)?,
                    #[cfg(not(target_os = "macos"))]
                    &PredefinedMenuItem::quit(app, None)?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "Edit",
                true,
                &[
                    &PredefinedMenuItem::undo(app, None)?,
                    &PredefinedMenuItem::redo(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::cut(app, None)?,
                    &PredefinedMenuItem::copy(app, None)?,
                    &PredefinedMenuItem::paste(app, None)?,
                    &PredefinedMenuItem::select_all(app, None)?,
                ],
            )?,
            #[cfg(target_os = "macos")]
            &Submenu::with_items(
                app,
                "View",
                true,
                &[&PredefinedMenuItem::fullscreen(app, None)?],
            )?,
            &window_menu,
            &help_menu,
        ],
    )
}
