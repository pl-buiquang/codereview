mod anchor;
pub mod commands;
mod cr_install;
pub mod db;
pub mod db_path;
pub mod deep_link;
pub mod error;
pub mod export;
pub mod gh;
pub mod git;
pub mod inbox;
mod path_env;
pub mod provider;
pub mod tools;
pub mod worktree;

use std::sync::Mutex;

use tauri::{Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;

use db::Db;

/// One-time install note for the `cr` CLI sidecar (shown as a frontend toast).
pub struct CrNote(pub Mutex<Option<String>>);

const CLOSE_TAB_MENU_ID: &str = "close_tab";
const CLOSE_ACTIVE_TAB_EVENT: &str = "close-active-tab";

fn handle_deep_link_url<R: tauri::Runtime>(app: &tauri::AppHandle<R>, raw_url: &str) {
    match deep_link::parse_deep_link(raw_url) {
        Ok(action) => {
            let _ = app.emit("deep-link-action", &action);
        }
        Err(err) => {
            eprintln!("[deep-link] failed to parse {raw_url:?}: {err}");
            let _ = app.emit("deep-link-error", &err);
        }
    }
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_focus();
        let _ = w.unminimize();
    }
}

#[tauri::command]
fn app_version() -> &'static str {
    env!("CODEREVIEW_VERSION")
}

#[tauri::command]
fn cr_install_note(note: tauri::State<CrNote>) -> Option<String> {
    note.0.lock().unwrap().take()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    path_env::ensure_login_path();
    let cr_note = cr_install::maybe_install().map(|r| {
        let verb = if r.updated { "updated" } else { "installed" };
        format!("cr CLI {verb} → {}", r.path.display())
    });
    tools::init();

    tauri::Builder::default()
        // single-instance must come before deep-link
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(url) = argv.get(1) {
                handle_deep_link_url(app, url);
            } else if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_focus();
                let _ = w.unminimize();
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
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
        .manage(CrNote(Mutex::new(cr_note)))
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

            // Handle deep links delivered while the app is already running
            // (macOS open-url Apple Events, Linux D-Bus activation).
            // The plugin buffers any URL received before this listener is
            // registered and replays it here, covering cold-start URLs too.
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    handle_deep_link_url(&handle, url.as_str());
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::repo::add_repository,
            commands::repo::list_repositories,
            commands::repo::remove_repository,
            commands::repo::list_branches,
            commands::repo::diff_refs,
            commands::repo::link_local_path,
            commands::repo::auto_link_repos,
            commands::review::create_review,
            commands::review::list_reviews,
            commands::review::get_review,
            commands::review::set_file_viewed,
            commands::review::set_panel_widths,
            commands::review::set_sidebar_collapsed,
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
            commands::export::export_vscode_review,
            commands::export::import_review,
            commands::editor::open_in_default_app,
            commands::editor::open_url,
            commands::gh::gh_auth_status,
            commands::gh::list_prs,
            commands::gh::pr_meta,
            commands::gh::pr_review_threads,
            commands::gh::reply_to_thread,
            commands::gh::set_pr_thread_resolved,
            commands::gh::check_environment,
            commands::gh::fetch_github_image,
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
            commands::chat::chat_send,
            commands::chat::chat_messages,
            commands::chat::chat_clear,
            commands::chat::set_chat_collapsed,
            cr_install_note,
            app_version,
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
