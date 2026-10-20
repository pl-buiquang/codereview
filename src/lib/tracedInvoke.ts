import { invoke as rawInvoke, type InvokeArgs, type InvokeOptions } from "@tauri-apps/api/core";
import { brief, log, logEnabled } from "./log";

/** Commands that must not trace themselves: the Settings log viewer polls them. */
const UNTRACED = new Set(["read_app_log", "log_path", "frontend_log", "get_log_level"]);

/**
 * User-initiated actions that change state (local DB, git, GitHub, agents): logged at info.
 * Everything else — reads, polling, autosaves, UI prefs — stays at debug, so a new command is
 * quiet by default.
 */
export const MAJOR_ACTIONS = new Set([
  // repositories
  "add_repository",
  "remove_repository",
  "link_local_path",
  "auto_link_repos",
  "set_repository_friendly_name",
  // reviews and comments
  "create_review",
  "new_review_on_target",
  "create_review_for_pr",
  "delete_review",
  "delete_reviews",
  "refresh_review",
  "update_review_pin",
  "reanchor_comments",
  "import_review",
  "export_review",
  "export_vscode_review",
  "add_comment",
  "add_file_comment",
  "add_file_view_comment",
  "delete_comment",
  "set_comment_resolved",
  "add_draft_reply",
  "delete_draft_reply",
  "publish_draft_reply",
  "publish_draft_replies",
  "publish_review",
  "publish_review_pending",
  "submit_pending_review",
  "discard_pending_review",
  // AI
  "generate_focus",
  "clear_focus",
  "set_focus_plan",
  "run_ai_review",
  "chat_send",
  "chat_clear",
  "board_overview_generate",
  "generate_live_session_summary",
  "prompt_set",
  "prompt_reset",
  // GitHub
  "set_pr_draft",
  "update_pr_branch",
  "set_pr_auto_merge",
  "reply_to_thread",
  "set_pr_thread_resolved",
  // inbox and board
  "engage_item",
  "unengage_item",
  "untrack_item",
  "retrack_item",
  "open_pr_review",
  "board_track",
  "board_track_pr",
  "board_discard",
  "board_pr_set_auto_merge",
  "board_pr_squash_merge",
  "board_pr_set_draft",
  "board_pr_update_branch",
  "board_pr_rerun_ci",
  "board_link_pr",
  "board_unlink",
  "board_archive_done",
  "board_open_review",
  "board_open_vscode",
  "board_open_shell",
  "board_open_agent",
  "board_meta_add",
  "board_meta_delete",
  // agent sessions, worktrees, terminals
  "review_open_agent",
  "resume_claude_session",
  "stop_claude_session",
  "worktree_gc_run",
  "worktree_sync",
  "worktree_open_terminal",
  "checkout_open_claude",
  "ensure_chat_worktree",
  "remove_worktree",
  "prune_worktrees",
  "open_in_vscode",
  // sync
  "sync_now",
  "reset_mirror",
  "fetch_repo",
  // git
  "delete_gone_branches",
  "git_checkout_branch",
  "git_checkout_remote_branch",
  "git_checkout_detached",
  "git_create_branch",
  "git_delete_branch",
  "git_rename_branch",
  "git_create_tag",
  "git_delete_tag",
  "git_stage",
  "git_unstage",
  "git_discard",
  "git_commit",
  "git_pull",
  "git_push",
  "git_stash_push",
  "git_stash_apply",
  "git_stash_pop",
  "git_stash_drop",
]);

/** Drop-in for `@tauri-apps/api/core::invoke` that logs every backend call: major actions at
 *  info (command and args), the outcome and duration of every call at debug, failures at warn. */
export async function invoke<T>(cmd: string, args?: InvokeArgs, options?: InvokeOptions): Promise<T> {
  const call = () => (options ? rawInvoke<T>(cmd, args, options) : args === undefined ? rawInvoke<T>(cmd) : rawInvoke<T>(cmd, args));
  if (UNTRACED.has(cmd) || !logEnabled("warn")) return call();
  const major = MAJOR_ACTIONS.has(cmd);
  const traceCall = logEnabled(major ? "info" : "debug");
  const traceResult = logEnabled("debug");
  const started = performance.now();
  if (traceCall) {
    const line = `→ ${cmd} ${args === undefined ? "" : brief(args)}`;
    if (major) log.info("invoke", line);
    else log.debug("invoke", line);
  }
  try {
    const result = await call();
    if (traceResult) log.debug("invoke", `← ${cmd} ok in ${Math.round(performance.now() - started)}ms`);
    return result;
  } catch (e) {
    log.warn("invoke", `← ${cmd} FAILED in ${Math.round(performance.now() - started)}ms: ${brief(e)}`);
    throw e;
  }
}
