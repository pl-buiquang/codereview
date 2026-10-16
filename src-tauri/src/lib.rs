mod anchor;
mod avatar;
pub mod board;
pub mod claude_sessions;
pub mod commands;
mod cr_install;
pub mod db;
pub mod db_path;
pub mod deep_link;
pub mod error;
pub mod export;
pub mod gh;
pub mod git;
pub mod graph;
pub mod inbox;
pub mod logging;
mod path_env;
pub mod provider;
pub mod prompts;
pub mod session_summaries;
pub mod terminal;
pub mod tools;
pub mod worktree;
pub mod worktree_gc;

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
            logging::warn(&format!("[deep-link] failed to parse {raw_url:?}: {err}"));
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

#[tauri::command]
fn log_path(dir: tauri::State<logging::LogDir>) -> String {
    logging::log_file_path(&dir.0).to_string_lossy().into_owned()
}

#[tauri::command]
fn read_app_log(max_lines: usize, dir: tauri::State<logging::LogDir>) -> Result<String, String> {
    logging::read_tail(&dir.0, max_lines).map_err(|e| format!("failed to read log: {e}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let t0 = std::time::Instant::now();
    path_env::ensure_login_path();
    let login_env_ms = t0.elapsed().as_millis();
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
        .setup(move |app| {
            // Ensure ~/.codereview/ exists for worktrees and future per-user data.
            if let Some(home) = dirs::home_dir() {
                let _ = std::fs::create_dir_all(home.join(".codereview"));
            }

            let db_path = match std::env::var_os("CODEREVIEW_DB") {
                Some(p) => std::path::PathBuf::from(p),
                None => {
                    let dir = app.path().app_data_dir()?;
                    std::fs::create_dir_all(&dir)?;
                    dir.join(db_path::db_filename())
                }
            };
            if let Some(parent) = db_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let conn = db::open(&db_path)?;
            commands::focus::fail_interrupted(&conn)?;
            app.manage(Db(Mutex::new(conn)));
            app.manage(db::DbPath(db_path.clone()));

            let log_dir = db_path
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            if let Err(e) = logging::init(&log_dir) {
                eprintln!("[logging] failed to open log file in {}: {e}", log_dir.display());
            }
            log::info!(
                "codereview starting (version {}, debug_assertions={}, db={})",
                env!("CODEREVIEW_VERSION"),
                cfg!(debug_assertions),
                db_path.display(),
            );
            log::info!(
                "startup: login env {login_env_ms}ms, setup done {}ms since launch",
                t0.elapsed().as_millis(),
            );
            app.manage(logging::LogDir(log_dir));

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
            log_path,
            read_app_log,
            commands::repo::add_repository,
            commands::repo::list_repositories,
            commands::repo::remove_repository,
            commands::repo::list_branches,
            commands::repo::diff_refs,
            commands::repo::link_local_path,
            commands::repo::auto_link_repos,
            commands::repo::set_repository_friendly_name,
            commands::focus::focus_hunks,
            commands::focus::get_focus,
            commands::focus::set_focus_plan,
            commands::focus::set_focus_level,
            commands::focus::set_hunk_override,
            commands::focus::set_file_override,
            commands::focus::clear_focus,
            commands::focus::generate_focus,
            commands::review::create_review,
            commands::review::list_reviews,
            commands::review::list_target_reviews,
            commands::review::new_review_on_target,
            commands::review::get_review,
            commands::review::set_file_viewed,
            commands::review::set_panel_widths,
            commands::review::set_sidebar_collapsed,
            commands::review::update_review,
            commands::review::delete_review,
            commands::review::find_closed_pr_reviews,
            commands::review::delete_reviews,
            commands::review::add_comment,
            commands::review::add_file_comment,
            commands::review::add_file_view_comment,
            commands::review::update_comment,
            commands::review::set_comment_tag,
            commands::review::delete_comment,
            commands::review::set_comment_resolved,
            commands::review::add_draft_reply,
            commands::review::update_draft_reply,
            commands::review::delete_draft_reply,
            commands::review::publish_draft_replies,
            commands::review::publish_draft_reply,
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
            commands::export::write_settings_file,
            commands::export::read_settings_file,
            commands::export::read_settings_mirror,
            commands::export::write_settings_mirror,
            commands::editor::open_in_default_app,
            commands::editor::open_url,
            commands::gh::gh_auth_status,
            commands::gh::list_prs,
            commands::gh::pr_meta,
            commands::gh::set_pr_draft,
            commands::gh::update_pr_branch,
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
            commands::board::board_refresh,
            commands::board::board_last_refresh_at,
            commands::board::board_list,
            commands::board::board_tracked_ids,
            commands::board::board_track,
            commands::board::board_track_pr,
            commands::board::board_discard,
            commands::board::board_pr_merge,
            commands::board::board_pr_set_draft,
            commands::board::board_pr_update_branch,
            commands::board::board_link_pr,
            commands::board::board_unlink,
            commands::board::board_archive_done,
            commands::board::board_open_review,
            commands::board::board_open_vscode,
            commands::board::board_open_shell,
            commands::board::board_open_agent,
            commands::sessions::list_claude_sessions,
            commands::sessions::focus_claude_session,
            commands::sessions::resume_claude_session,
            commands::sessions::stop_claude_session,
            commands::sessions::board_card_sessions,
            commands::sessions::review_agent_sessions,
            commands::sessions::review_open_agent,
            commands::board::board_meta_add,
            commands::board::board_meta_update,
            commands::board::board_meta_delete,
            commands::board::board_meta_tags,
            commands::board::board_card_for_pr,
            commands::overview::board_overview_get,
            commands::overview::board_overview_generate,
            commands::prompts::prompts_list,
            commands::prompts::prompt_set,
            commands::prompts::prompt_reset,
            commands::chat::chat_send,
            commands::chat::chat_messages,
            commands::chat::chat_clear,
            commands::chat::chat_worktree,
            commands::worktree::worktree_gc_run,
            commands::worktree::worktree_gc_last,
            commands::worktree::worktree_sync,
            commands::worktree::worktree_open_terminal,
            commands::chat::ensure_chat_worktree,
            commands::chat::set_chat_collapsed,
            commands::worktree::list_worktrees,
            commands::worktree::remove_worktree,
            commands::worktree::prune_worktrees,
            commands::worktree::open_in_vscode,
            commands::graph::graph_log,
            commands::graph::list_refs,
            commands::graph::commit_avatar,
            commands::graph::commit_detail,
            commands::graph::delete_gone_branches,
            commands::graph::list_gone_branches,
            commands::graph::commit_file_diff,
            commands::graph::worktree_status,
            commands::graph::worktree_file_diff,
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
