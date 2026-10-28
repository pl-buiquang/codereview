pub mod ai_gate;
pub mod ai_usage;
mod anchor;
mod avatar;
pub mod board;
pub mod board_monitor;
pub mod claude_sessions;
pub mod commands;
mod cr_install;
pub mod db;
pub mod db_path;
pub mod deep_link;
mod definitions;
mod diff_cache;
pub mod error;
pub mod export;
pub mod gh;
pub mod git;
pub mod graph;
pub mod inbox;
pub mod logging;
pub mod mirror;
mod path_env;
pub mod provider;
pub mod prompts;
pub mod repo_sync;
pub mod routines;
pub mod stack_rebase;
pub mod session_summaries;
pub mod sync;
pub mod terminal;
pub mod tools;
pub mod watch;
pub mod worktree;
pub mod worktree_gc;

use std::sync::Mutex;

use tauri::{Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;

use db::Db;

/// One-time install note for the `cr` CLI sidecar (shown as a frontend toast).
pub struct CrNote(pub Mutex<Option<String>>);

const CLOSE_TAB_MENU_ID: &str = "close_tab";
#[cfg(target_os = "macos")]
const QUIT_MENU_ID: &str = "quit";
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

#[tauri::command]
fn get_log_level() -> &'static str {
    logging::current_level()
}

#[tauri::command]
fn set_log_level(level: String, dir: tauri::State<logging::LogDir>) -> Result<(), String> {
    let parsed = logging::parse_level(&level).ok_or_else(|| format!("unknown log level {level:?}"))?;
    logging::set_level(&dir.0, parsed);
    Ok(())
}

/// A log line from the frontend (already filtered by its level).
#[tauri::command]
fn frontend_log(level: String, target: String, message: String) {
    logging::frontend(&level, &target, &message);
}

/// Commands that must not trace themselves: the Settings log viewer polls
/// `read_app_log` every few seconds and every UI log line goes through
/// `frontend_log`, so tracing them would feed the log with its own reads.
const UNTRACED_COMMANDS: &[&str] = &["read_app_log", "log_path", "frontend_log", "get_log_level"];

/// Wrap the invoke handler so every command call is logged at debug level.
fn traced<R: tauri::Runtime>(
    handler: impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    move |invoke| {
        let cmd = invoke.message.command();
        if !UNTRACED_COMMANDS.contains(&cmd) && log::log_enabled!(log::Level::Debug) {
            log::debug!(target: "cmd", "{cmd}");
        }
        handler(invoke)
    }
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
        .plugin(tauri_plugin_notification::init())
        .menu(app_menu)
        .on_menu_event(|app, event| {
            if event.id() == CLOSE_TAB_MENU_ID {
                let _ = app.emit(CLOSE_ACTIVE_TAB_EVENT, ());
            }
            // The native Quit item (`terminate:`) bypasses `ExitRequested` on macOS, so it would kill
            // running routines without asking: ⌘Q goes through the same prompt instead.
            #[cfg(target_os = "macos")]
            if event.id() == QUIT_MENU_ID {
                match app.state::<routines::manager::RoutineManager>().active_count(app) {
                    0 => app.exit(0),
                    active => show_quit_prompt(app, active),
                }
            }
        })
        .manage(CrNote(Mutex::new(cr_note)))
        .manage(watch::Watchers::default())
        .manage(routines::manager::RoutineManager::default())
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
            routines::seed(&conn)?;
            routines::adopt_builtin_keys(&conn)?;
            routines::adopt_prompt_overrides(&conn)?;
            routines::manager::mark_interrupted(&conn, &db_path)?;
            app.manage(Db(Mutex::new(conn)));
            ai_usage::init(app.handle().clone());
            app.manage(db::DbPath(db_path.clone()));
            routines::manager::RoutineManager::start_ticker(app.handle().clone());
            routines::manager::RoutineManager::adopt_detached(app.handle());
            app.manage(sync::SyncEngine::start(app.handle().clone()));

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
        .invoke_handler(traced(tauri::generate_handler![
            ai_usage::ai_usage_summary,
            ai_usage::ai_usage_runs,
            log_path,
            read_app_log,
            get_log_level,
            set_log_level,
            frontend_log,
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
            commands::ai_review::run_ai_review,
            commands::ai_review::ai_review_status,
            commands::routines::routine_list,
            commands::routines::routine_save,
            commands::routines::routine_delete,
            commands::routines::routine_reset,
            commands::routines::routine_list_for_context,
            commands::routines::routine_launch,
            commands::routines::routine_run_list,
            commands::routines::routine_run_events,
            commands::routines::routine_run_stop,
            commands::routines::routine_run_relaunch,
            commands::routines::routine_max_concurrency,
            commands::routines::routine_set_max_concurrency,
            commands::routines::routine_settings,
            commands::routines::routine_set_settings,
            commands::review::create_review,
            commands::review::list_reviews,
            commands::review::list_target_reviews,
            commands::review::new_review_on_target,
            commands::review::get_review,
            commands::review::set_file_viewed,
            commands::review::set_panel_widths,
            commands::review::set_sidebar_collapsed,
            commands::review::set_pr_details_collapsed,
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
            commands::review::find_definition,
            commands::review::publish_review,
            commands::review::publish_review_pending,
            commands::review::submit_pending_review,
            commands::review::discard_pending_review,
            commands::review::refresh_review,
            commands::review::update_review_pin,
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
            commands::gh::mirror_pr_meta,
            commands::gh::set_pr_draft,
            commands::gh::set_pr_label,
            commands::gh::set_pr_reviewer,
            commands::gh::repo_labels,
            commands::gh::repo_assignable_users,
            commands::gh::pr_checks,
            commands::gh::rerun_ci_runs,
            commands::gh::rerun_ci_job,
            commands::gh::update_pr_branch,
            commands::gh::set_pr_auto_merge,
            commands::gh::pr_review_threads,
            commands::gh::mirror_pr_threads,
            commands::gh::reply_to_thread,
            commands::gh::set_pr_thread_resolved,
            commands::gh::set_comment_reaction,
            commands::gh::check_environment,
            commands::gh::fetch_github_image,
            commands::inbox::list_inbox,
            commands::inbox::list_inbox_archive,
            commands::conversations::list_conversations,
            commands::conversations::set_conversation_pinned,
            commands::conversations::set_conversation_seen,
            commands::conversations::reply_to_discussion,
            commands::inbox::archive_item,
            commands::inbox::forget_item,
            commands::inbox::restore_item,
            commands::inbox::open_pr_review,
            commands::inbox::inbox_meta,
            commands::inbox::mark_item_viewed,
            commands::inbox::mark_item_unread,
            commands::board::board_last_refresh_at,
            commands::board::board_list,
            commands::board::board_tracked_ids,
            commands::board::board_track,
            commands::board::board_track_pr,
            commands::board::board_discard,
            commands::board::board_pr_set_auto_merge,
            commands::board::board_pr_squash_merge,
            commands::board::board_pr_set_draft,
            commands::board::board_pr_update_branch,
            commands::stack_rebase::board_stack_rebase_preview,
            commands::stack_rebase::board_stack_rebase,
            commands::board::board_pr_rerun_ci,
            commands::board::board_link_pr,
            commands::board::board_unlink,
            commands::board::board_archive_done,
            commands::board::board_monitor_log,
            commands::board::board_open_review,
            commands::board::board_open_vscode,
            commands::board::board_open_shell,
            commands::board::board_open_agent,
            commands::issue_session::board_open_issue_agent,
            commands::sessions::list_claude_sessions,
            commands::sessions::generate_live_session_summary,
            commands::theme::theme_suggest,
            commands::sessions::focus_claude_session,
            commands::sessions::resume_claude_session,
            commands::sessions::stop_claude_session,
            commands::sessions::review_agent_sessions,
            commands::sessions::review_open_agent,
            commands::board::board_meta_add,
            commands::board::board_meta_update,
            commands::board::board_meta_delete,
            commands::board::board_meta_tags,
            commands::board::board_card_for_pr,
            commands::overview::board_overview_get,
            commands::overview::board_overview_generate,
            ai_gate::set_ai_enabled,
            commands::prompts::prompts_list,
            commands::prompts::prompt_set,
            commands::prompts::prompt_reset,
            commands::chat::chat_send,
            commands::chat::chat_messages,
            commands::chat::chat_clear,
            commands::chat::chat_worktree,
            commands::worktree::worktree_gc_run,
            commands::worktree::worktree_gc_last,
            commands::worktree::worktree_gc_reload,
            commands::worktree::worktree_sync,
            commands::worktree::worktree_open_terminal,
            commands::worktree::checkout_open_claude,
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
            commands::repo_sync::fetch_repo,
            commands::sync::sync_hint,
            commands::sync::sync_window_state,
            commands::sync::sync_configure,
            commands::sync::sync_status,
            commands::sync::sync_now,
            commands::sync::reset_mirror,
            commands::repo_sync::repo_sync_state,
            commands::repo_sync::watch_acquire,
            commands::repo_sync::watch_release,
            commands::graph::commit_file_diff,
            commands::graph::worktree_status,
            commands::graph::worktree_file_diff,
            commands::graph::git_checkout_branch,
            commands::graph::git_checkout_remote_branch,
            commands::graph::git_checkout_detached,
            commands::graph::git_create_branch,
            commands::graph::git_delete_branch,
            commands::graph::git_rename_branch,
            commands::graph::git_create_tag,
            commands::graph::git_delete_tag,
            commands::graph::git_stage,
            commands::graph::git_unstage,
            commands::graph::git_discard,
            commands::graph::git_commit,
            commands::graph::git_pull,
            commands::graph::git_push,
            commands::graph::git_stash_push,
            commands::graph::git_stash_list,
            commands::graph::git_stash_apply,
            commands::graph::git_stash_pop,
            commands::graph::git_stash_drop,
            commands::graph::git_stash_diff,
            cr_install_note,
            app_version,
        ]))
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            match event {
                // A user-initiated quit (no exit code) with routines running asks first.
                // `app.exit(..)` and the updater's restart carry a code and go straight through.
                tauri::RunEvent::ExitRequested { code: None, api, .. } => {
                    let manager = app.state::<routines::manager::RoutineManager>();
                    let active = manager.active_count(app);
                    if active == 0 {
                        return;
                    }
                    api.prevent_exit();
                    show_quit_prompt(app, active);
                }
                // Routine runs lead their own process groups, so they'd outlive us unless killed here.
                tauri::RunEvent::Exit => app.state::<routines::manager::RoutineManager>().kill_all(app),
                _ => {}
            }
        });
}

/// Ask what to do with the `active` routine runs a quit would interrupt: stop them, leave them
/// running to be re-adopted on the next launch, or cancel the quit.
fn show_quit_prompt(app: &tauri::AppHandle, active: usize) {
    let manager = app.state::<routines::manager::RoutineManager>();
    if !manager.begin_quit_prompt() {
        return;
    }
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogResult};
    const STOP_AND_QUIT: &str = "Stop and quit";
    const LEAVE_RUNNING: &str = "Leave running";
    let handle = app.clone();
    app.dialog()
        .message(format!(
            "{active} routine run{} in progress. Stop {}, or leave {} running and pick {} up on the next launch?",
            if active == 1 { " is" } else { "s are" },
            if active == 1 { "it" } else { "them" },
            if active == 1 { "it" } else { "them" },
            if active == 1 { "it" } else { "them" },
        ))
        .title("Routines are running")
        .buttons(MessageDialogButtons::YesNoCancelCustom(
            STOP_AND_QUIT.into(),
            LEAVE_RUNNING.into(),
            "Cancel".into(),
        ))
        .show_with_result(move |choice| {
            let manager = handle.state::<routines::manager::RoutineManager>();
            manager.end_quit_prompt();
            match choice {
                MessageDialogResult::Custom(c) if c == STOP_AND_QUIT => handle.exit(0),
                MessageDialogResult::Custom(c) if c == LEAVE_RUNNING => {
                    manager.detach_all(&handle);
                    handle.exit(0);
                }
                _ => {}
            }
        });
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
                    &MenuItem::with_id(
                        app,
                        QUIT_MENU_ID,
                        format!("Quit {}", pkg_info.name),
                        true,
                        Some("CmdOrCtrl+Q"),
                    )?,
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
