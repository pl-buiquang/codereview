use std::process;
use std::sync::Mutex;

use clap::{Parser, Subcommand};

use codereview_lib::board;
use codereview_lib::commands::{board as board_cmd, export, focus, inbox, repo, review};
use codereview_lib::db::{self, Db};
use codereview_lib::db_path::resolve_db_path;
use codereview_lib::error::{AppError, AppResult};
use codereview_lib::gh::{self, GhRepo};
use codereview_lib::provider::provider_for;
use codereview_lib::sync::{self, Mode, Scope, SyncReport, SyncRequest};
use codereview_lib::tools;

#[derive(Parser)]
#[command(
    name = "cr",
    about = "CodeReview CLI — browse and manage PR reviews",
    version = env!("CODEREVIEW_VERSION"),
)]
struct Cli {
    /// Path to the SQLite database (overrides CODEREVIEW_DB env and platform default)
    #[arg(long, global = true, env = "CODEREVIEW_DB")]
    db: Option<String>,

    /// Output as JSON instead of human-readable text
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List reviews, optionally filtered by repo
    List {
        /// Filter to a specific repository ID
        #[arg(long)]
        repo: Option<i64>,
    },
    /// Show a review's detail
    Show {
        review_id: i64,
    },
    /// Print the diff for a review
    Diff {
        review_id: i64,
    },
    /// Print the path of a checkout of the review's head branch, creating a worktree if none holds it
    Worktree {
        review_id: i64,
    },
    /// Create a new review
    Create {
        #[command(subcommand)]
        target: CreateTarget,
    },
    /// Start another draft review on the same PR / local diff as <review-id>,
    /// even if it already has an unpublished one (e.g. a second pass with a
    /// different kind of comment)
    New {
        review_id: i64,
    },
    /// Update a review's body or verdict
    Update {
        review_id: i64,
        /// Set the review body/summary text
        #[arg(long)]
        body: Option<String>,
        /// Set the verdict: approve, comment, or request_changes
        #[arg(long)]
        verdict: Option<String>,
    },
    /// Delete a draft review
    Delete {
        review_id: i64,
    },
    /// Comment operations
    Comment {
        #[command(subcommand)]
        action: CommentAction,
    },
    /// Export a review
    Export {
        review_id: i64,
        /// Output format: markdown, md, or json
        #[arg(long, default_value = "markdown")]
        format: String,
        /// Write to file instead of stdout
        #[arg(long, short)]
        output: Option<String>,
    },
    /// Repository management
    Repo {
        #[command(subcommand)]
        action: RepoAction,
    },
    /// Inbox operations
    Inbox {
        #[command(subcommand)]
        action: InboxAction,
    },
    /// Open the CodeReview app via deep link
    Open {
        #[command(subcommand)]
        target: OpenTarget,
    },
    /// Focus overlay: AI-tiered hunks shown as a filter over the review's diff
    Focus {
        #[command(subcommand)]
        action: FocusAction,
    },
    /// List GitHub review threads for a PR (for drafting replies with `cr comment draft-reply`)
    Threads {
        owner: String,
        name: String,
        number: i64,
    },
    /// Kanban board operations
    Board {
        #[command(subcommand)]
        action: BoardAction,
    },
    /// Sync the GitHub mirror (Inbox + Board): discovery, then refetch what changed
    Sync {
        /// Refetch everything, ignoring change keys; also fetches every local clone
        #[arg(long, conflicts_with = "light")]
        full: bool,
        /// Only check known items for changes (no search for new ones)
        #[arg(long)]
        light: bool,
    },
}

#[derive(Subcommand)]
enum BoardAction {
    /// List board cards, grouped by lane
    List {
        /// Filter to one lane: backlog, in_progress, submitted, reviews, done
        #[arg(long)]
        lane: Option<String>,
    },
    /// Show one card
    Show {
        /// GitHub URL or owner/repo#n
        reference: Option<String>,
        /// Resolve the card from the current repo + branch instead
        #[arg(long)]
        here: bool,
    },
    /// Track an issue or PR on the board
    Track {
        /// GitHub URL or owner/repo#n
        reference: String,
    },
    /// Sync the board from GitHub
    #[command(alias = "refresh")]
    Sync,
    /// Manually link a PR to a backlog issue card
    Link {
        /// The backlog issue's card id (see `cr board list --json`)
        issue_id: String,
        /// The PR to link: GitHub URL or owner/repo#n
        pr_reference: String,
    },
    /// Card metadata (plan/spec files, links, notes, Claude sessions)
    Meta {
        #[command(subcommand)]
        action: BoardMetaAction,
    },
}

#[derive(Subcommand)]
enum BoardMetaAction {
    /// Attach a metadata item to a card
    Add {
        /// GitHub URL or owner/repo#n
        reference: Option<String>,
        /// Resolve the card from the current repo + branch instead
        #[arg(long)]
        here: bool,
        /// file, link, note, or claude_session
        #[arg(long)]
        kind: String,
        /// Path (file/link kinds), URL, note text, or Claude session id
        /// (optional for a note that has --tag set)
        #[arg(long, default_value = "")]
        value: String,
        /// Freeform tag (e.g. "plan", "spec")
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        label: Option<String>,
        /// Working directory the session was started from (claude_session only)
        #[arg(long)]
        cwd: Option<String>,
        /// Track the card first if it isn't already tracked
        #[arg(long)]
        track: bool,
    },
    /// List a card's metadata
    List {
        /// GitHub URL or owner/repo#n
        reference: Option<String>,
        /// Resolve the card from the current repo + branch instead
        #[arg(long)]
        here: bool,
    },
    /// Delete a metadata item by id
    Rm { id: i64 },
}

#[derive(Subcommand)]
enum CreateTarget {
    /// Open the review of a local diff between two refs: its unpublished
    /// review if any, else a new one
    Local {
        repo_id: i64,
        base_ref: String,
        head_ref: String,
        /// Use three-dot diff (merge-base semantics, like GitHub PRs)
        #[arg(long)]
        three_dot: bool,
        /// No-op, kept for old scripts: an unpublished review is always reused
        #[arg(long, hide = true)]
        reuse_draft: bool,
    },
    /// Open the review of a GitHub PR: its unpublished review if any, else a new one.
    /// Accepts either a GitHub URL (https://github.com/owner/name/pull/123)
    /// or three positional args: owner name number.
    Pr {
        /// GitHub PR URL or repo owner
        owner_or_url: String,
        /// Repo name (required when not using a URL)
        name: Option<String>,
        /// PR number (required when not using a URL)
        number: Option<i64>,
        /// No-op, kept for old scripts: an unpublished review is always reused
        #[arg(long, hide = true)]
        reuse_draft: bool,
    },
}

#[derive(Subcommand)]
enum FocusAction {
    /// List the review's current hunks (id, file, header, +/- counts, rule tier, preview)
    Hunks { review_id: i64 },
    /// Print one hunk's full text (header + body)
    Hunk { review_id: i64, hunk_id: String },
    /// Write a focus plan, replacing any previous one (overrides are kept)
    Set {
        review_id: i64,
        /// Plan JSON file ({"summary", "hunks": [{"id","tier","reason","note"}]}); "-" for stdin
        #[arg(long)]
        file: String,
        /// Level to show: off, thorough, focus, or critical
        #[arg(long)]
        level: Option<String>,
    },
    /// Show the review's focus
    Show { review_id: i64 },
    /// Delete the review's focus
    Clear { review_id: i64 },
}

const FOCUS_HUNK_WARN: usize = 500;

/// `--tag` values map "none" to no tag; `ai`/`ai:*`/`local` pass through as-is
/// (validated downstream by `add_comment_impl`/`set_comment_tag_impl`).
fn parse_tag(tag: &str) -> Option<&str> {
    (tag != "none").then_some(tag)
}

#[derive(Subcommand)]
enum CommentAction {
    /// Add a line-level comment
    Add {
        review_id: i64,
        /// File path (relative to repo root)
        #[arg(long)]
        file: String,
        /// Line number in the diff
        #[arg(long)]
        line: i64,
        /// Which side: LEFT (old/deleted) or RIGHT (new/added)
        #[arg(long, default_value = "RIGHT")]
        side: String,
        /// Comment body text
        #[arg(long)]
        body: String,
        /// ai, ai:explain, ai:review, ai:codex, local, or none. Tagged comments are never published. Defaults
        /// to local: a comment made from the CLI stays off GitHub until you
        /// untag it or edit it in the app.
        #[arg(long, default_value = "local")]
        tag: String,
    },
    /// Add a whole-file comment (not anchored to a line)
    AddFile {
        review_id: i64,
        /// File path (relative to repo root)
        #[arg(long)]
        file: String,
        /// Comment body text
        #[arg(long)]
        body: String,
        /// ai, ai:explain, ai:review, ai:codex, local, or none. See `comment add --tag`.
        #[arg(long, default_value = "local")]
        tag: String,
    },
    /// Update a comment's body
    Update {
        comment_id: i64,
        /// New body text
        #[arg(long)]
        body: String,
    },
    /// Set or clear a comment's tag
    Tag {
        comment_id: i64,
        /// ai, ai:explain, ai:review, ai:codex, local, or none
        #[arg(long)]
        tag: String,
    },
    /// Delete a comment
    Delete {
        comment_id: i64,
    },
    /// Mark a comment thread as resolved
    Resolve {
        comment_id: i64,
    },
    /// Mark a comment thread as unresolved
    Unresolve {
        comment_id: i64,
    },
    /// Reply to a local comment thread (existing review comments, not GitHub threads)
    Reply {
        review_id: i64,
        /// Root comment ID to reply to
        #[arg(long)]
        parent_id: i64,
        /// Reply body text
        #[arg(long)]
        body: String,
        /// ai, ai:explain, ai:review, ai:codex, local, or none. See `comment add --tag`.
        #[arg(long, default_value = "local")]
        tag: String,
    },
    /// Draft a local reply to an existing GitHub review thread comment. Not
    /// posted to GitHub until `cr comment publish-replies` is run.
    DraftReply {
        review_id: i64,
        /// databaseId of the GitHub thread's root comment (see `cr threads --json`)
        #[arg(long)]
        thread_comment_id: i64,
        /// File path the thread is anchored to
        #[arg(long)]
        file: String,
        /// Which side: LEFT (old/deleted) or RIGHT (new/added)
        #[arg(long, default_value = "RIGHT")]
        side: String,
        /// Line number the thread is anchored to
        #[arg(long)]
        line: i64,
        /// Reply body text
        #[arg(long)]
        body: String,
    },
    /// Publish all draft replies for a review to their GitHub threads
    PublishReplies {
        review_id: i64,
    },
    /// Publish a single draft reply to its GitHub thread
    PublishReply {
        comment_id: i64,
    },
}

#[derive(Subcommand)]
enum RepoAction {
    /// List registered repositories
    List,
    /// Add a git repository
    Add { path: String },
    /// Remove a repository
    Remove { id: i64 },
}

#[derive(Subcommand)]
enum InboxAction {
    /// List the New queue
    List,
    /// Archive an inbox item (URL or `owner/repo#n`)
    Archive { reference: String },
    /// Archive an inbox item for good: a new reason won't bring it back
    Forget { reference: String },
    /// Bring an archived item back to New
    Restore { reference: String },
    /// Sync the inbox from GitHub
    #[command(alias = "refresh")]
    Sync,
}

#[derive(Subcommand)]
enum OpenTarget {
    /// Open an existing review by ID
    Review { review_id: i64 },
    /// Open/create a review for a GitHub PR
    Pr { owner: String, name: String, number: i64 },
    /// Open/create a review for a local diff
    Diff {
        repo_id: i64,
        base_ref: String,
        head_ref: String,
        /// Use three-dot diff (merge-base semantics, default: true)
        #[arg(long)]
        three_dot: bool,
    },
    /// Open a repo's commit graph, focused on a branch/tag (default: current branch of PATH or cwd)
    #[command(alias = "tree")]
    Graph {
        /// Directory inside the repo or worktree (default: cwd)
        path: Option<String>,
        /// Branch, remote branch or tag to focus (default: current branch)
        #[arg(long = "ref", conflicts_with = "no_ref")]
        ref_name: Option<String>,
        /// Open the graph without focusing any ref
        #[arg(long)]
        no_ref: bool,
        /// Registered repository ID (skips detection from PATH)
        #[arg(long)]
        repo: Option<i64>,
    },
    /// Navigate to the home tab
    Home {
        /// Section: inbox, reviews, archive, repositories
        section: Option<String>,
    },
    /// Open a raw codereview:// URL
    Url { url: String },
}

/// Parse a GitHub PR URL into (owner, name, number).
/// Accepts full URLs: https://github.com/owner/name/pull/123[anything]
fn parse_github_pr_url(url: &str) -> Option<(String, String, i64)> {
    let path = url.split("github.com/").nth(1)?;
    let parts: Vec<&str> = path.splitn(4, '/').collect();
    if parts.len() >= 4 && parts[2] == "pull" {
        let number: i64 = parts[3]
            .split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok()?;
        Some((parts[0].to_string(), parts[1].to_string(), number))
    } else {
        None
    }
}

/// Resolve the current repo + branch to a `owner/name#number` reference: the
/// branch's open PR if it has one, else an `issue-<n>`-named branch's issue.
fn resolve_here() -> AppResult<String> {
    let cwd = std::env::current_dir().map_err(|e| AppError::Other(format!("cannot read cwd: {e}")))?;
    let remote = codereview_lib::git::remote_info(&cwd);
    let (owner, name) = match (remote.owner, remote.name) {
        (Some(o), Some(n)) => (o, n),
        _ => return Err(AppError::Other("not inside a repo clone with a GitHub origin remote".into())),
    };

    let ctx = GhRepo::Local(cwd.clone());
    if let Ok(out) = gh::run_gh(&ctx, &["pr", "view", "--json", "number"]) {
        if let Some(number) = serde_json::from_str::<serde_json::Value>(&out)
            .ok()
            .and_then(|v| v.get("number").and_then(|n| n.as_i64()))
        {
            return Ok(format!("{owner}/{name}#{number}"));
        }
    }

    let branch = codereview_lib::git::run_git(&cwd, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|s| s.trim().to_string())
        .map_err(|e| AppError::Other(format!("cannot read current branch: {e}")))?;
    if let Some(number) = board::issue_number_from_branch(&branch) {
        return Ok(format!("{owner}/{name}#{number}"));
    }

    Err(AppError::Other(format!(
        "branch '{branch}' has no open PR and doesn't follow the issue-<n> naming convention"
    )))
}

struct GraphTarget {
    repo_id: i64,
    ref_name: Option<String>,
    worktree: Option<String>,
}

/// Resolve which registered repo / worktree / ref `cr open graph` should show.
fn resolve_graph_target(
    db: &Db,
    path: Option<String>,
    ref_name: Option<String>,
    no_ref: bool,
    repo_id: Option<i64>,
    json: bool,
) -> AppResult<GraphTarget> {
    let dir = match path {
        Some(p) => std::path::PathBuf::from(p),
        None => std::env::current_dir().map_err(|e| AppError::Other(format!("cannot read cwd: {e}")))?,
    };
    let not_repo = || AppError::NotARepo(dir.display().to_string());
    let git = |args: &[&str]| codereview_lib::git::run_git(&dir, args).map(|s| s.trim().to_string());

    let toplevel = std::path::PathBuf::from(git(&["rev-parse", "--show-toplevel"]).map_err(|_| not_repo())?);
    let worktrees = codereview_lib::worktree::list_worktrees(&toplevel)?;
    let main_path = worktrees
        .iter()
        .find(|w| w.is_main)
        .map(|w| w.path.clone())
        .ok_or_else(not_repo)?;
    let worktree = (toplevel != std::path::Path::new(&main_path)).then(|| toplevel.display().to_string());

    let repo_id = match repo_id {
        Some(id) => id,
        None => {
            let conn = db.0.lock().unwrap();
            let known = repo::list_repositories_impl(&conn)?
                .into_iter()
                .find(|r| r.local_path.as_deref() == Some(main_path.as_str()));
            match known {
                Some(r) => r.id,
                None => {
                    let r = repo::add_repository_impl(&conn, &main_path)?;
                    if !json {
                        eprintln!("Registered repository #{} ({main_path})", r.id);
                    }
                    r.id
                }
            }
        }
    };

    let ref_name = if no_ref {
        None
    } else if ref_name.is_some() {
        ref_name
    } else {
        codereview_lib::worktree::current_branch(&toplevel).or_else(|| {
            git(&["describe", "--tags", "--exact-match"]).ok().filter(|t| !t.is_empty())
        })
    };

    Ok(GraphTarget { repo_id, ref_name, worktree })
}

fn resolve_reference(reference: Option<String>, here: bool) -> AppResult<String> {
    match (reference, here) {
        (Some(r), false) => Ok(r),
        (None, true) => resolve_here(),
        (Some(_), true) => Err(AppError::Other("pass either a reference or --here, not both".into())),
        (None, false) => Err(AppError::Other("pass a reference (URL or owner/repo#n) or --here".into())),
    }
}

fn main() {
    let cli = Cli::parse();
    tools::init();

    let db_path = resolve_db_path(cli.db.as_deref());
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = match db::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: failed to open database at {}: {e}", db_path.display());
            process::exit(1);
        }
    };
    let db = Db(Mutex::new(conn));

    if let Err(e) = run(cli.cmd, cli.json, &db) {
        eprintln!("error: {e}");
        process::exit(1);
    }
}

fn run(cmd: Cmd, json: bool, db: &Db) -> AppResult<()> {
    match cmd {
        Cmd::List { repo: repo_id } => {
            let conn = db.0.lock().unwrap();
            let reviews = review::list_reviews_impl(&conn, repo_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&reviews)?);
            } else {
                if reviews.is_empty() {
                    println!("No reviews found.");
                    return Ok(());
                }
                println!(
                    "{:<6} {:<25} {:<30} {:<10} {:<8} {:<10}",
                    "ID", "REPO", "TARGET", "VERDICT", "CMTS", "STATUS"
                );
                for s in &reviews {
                    let target = if let Some(n) = s.target.github_pr_number {
                        format!("PR #{n}")
                    } else {
                        s.target.title.clone()
                    };
                    let target_display = if target.len() > 28 {
                        format!("{}...", &target[..25])
                    } else {
                        target
                    };
                    let verdict = s.review.event.as_deref().unwrap_or("-");
                    println!(
                        "{:<6} {:<25} {:<30} {:<10} {:<8} {:<10}",
                        s.review.id,
                        truncate(&s.repo_label, 23),
                        target_display,
                        verdict,
                        s.comment_count,
                        s.review.status,
                    );
                }
            }
        }

        Cmd::Show { review_id } => {
            let conn = db.0.lock().unwrap();
            let detail = review::load_detail(&conn, review_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&detail)?);
            } else {
                let label = review::repo_label(&conn, detail.target.repo_id)?;
                println!("Review #{} — {}", detail.review.id, detail.target.title);
                println!("Repo:    {label}");
                println!("Kind:    {}", detail.target.kind);
                println!("Base:    {}", detail.target.base_ref);
                println!("Head:    {}", detail.target.head_ref);
                println!("Status:  {}", detail.review.status);
                println!(
                    "Verdict: {}",
                    detail.review.event.as_deref().unwrap_or("(none)")
                );
                if !detail.review.body.is_empty() {
                    println!("\n## Summary\n{}", detail.review.body);
                }
                let root_comments: Vec<_> = detail
                    .comments
                    .iter()
                    .filter(|c| c.parent_id.is_none() && c.github_thread_root_id.is_none())
                    .collect();
                if !root_comments.is_empty() {
                    println!("\n## Comments ({} total)\n", root_comments.len());
                    for c in root_comments {
                        let loc = if c.subject_type == "file" {
                            c.file_path.clone()
                        } else {
                            format!("{}:{} ({})", c.file_path, c.line, c.side)
                        };
                        let tag = c.tag.as_deref().map(|t| format!(" [{t}]")).unwrap_or_default();
                        println!("  [#{}]{tag} {}", c.id, loc);
                        for line in c.body.lines() {
                            println!("        {line}");
                        }
                        let replies: Vec<_> = detail
                            .comments
                            .iter()
                            .filter(|r| r.parent_id == Some(c.id))
                            .collect();
                        for r in replies {
                            println!("    ↳ [#{}] {}", r.id, r.body.lines().next().unwrap_or(""));
                        }
                        println!();
                    }
                }
                let draft_replies: Vec<_> = detail
                    .comments
                    .iter()
                    .filter(|c| c.github_thread_root_id.is_some())
                    .collect();
                if !draft_replies.is_empty() {
                    println!(
                        "\n## Draft replies to GitHub threads ({} total, not yet published)\n",
                        draft_replies.len()
                    );
                    for c in draft_replies {
                        println!(
                            "  [#{}] -> thread comment {} ({}:{})",
                            c.id,
                            c.github_thread_root_id.unwrap_or(0),
                            c.file_path,
                            c.line,
                        );
                        for line in c.body.lines() {
                            println!("        {line}");
                        }
                        println!();
                    }
                }
            }
        }

        Cmd::Diff { review_id } => {
            let conn = db.0.lock().unwrap();
            let diff = review::review_diff_impl(&conn, review_id)?;
            print!("{diff}");
        }

        Cmd::Worktree { review_id } => {
            let detail = {
                let conn = db.0.lock().unwrap();
                review::load_detail(&conn, review_id)?
            };
            let local_path = detail.local_path.as_deref().ok_or_else(|| {
                AppError::Other("this review's repo has no local clone; link one in the app".into())
            })?;
            let path = codereview_lib::worktree::ensure_branch_worktree(
                std::path::Path::new(local_path),
                &detail.target,
                detail.review.head_sha.as_deref(),
            )?;
            if json {
                println!("{}", serde_json::json!({ "path": path.display().to_string() }));
            } else {
                println!("{}", path.display());
            }
        }

        Cmd::Create { target } => match target {
            CreateTarget::Local {
                repo_id,
                base_ref,
                head_ref,
                three_dot,
                reuse_draft: _,
            } => {
                let conn = db.0.lock().unwrap();
                let repos = repo::list_repositories_impl(&conn)?;
                let repo_row = repos.iter().find(|r| r.id == repo_id).ok_or_else(|| {
                    codereview_lib::error::AppError::Other(format!(
                        "repository {repo_id} not found"
                    ))
                })?;
                let repo_path = repo_row.local_path.as_deref().ok_or_else(|| {
                    codereview_lib::error::AppError::Other(
                        "creating a local review requires a local clone".into(),
                    )
                })?;
                let r = review::create_review_impl(
                    &conn,
                    repo_id,
                    repo_path,
                    &base_ref,
                    &head_ref,
                    three_dot,
                    false,
                )?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&r)?);
                } else {
                    println!("Review #{}", r.id);
                }
            }
            CreateTarget::Pr {
                owner_or_url,
                name,
                number,
                reuse_draft: _,
            } => {
                let (owner, repo_name, pr_number) =
                    if owner_or_url.contains("github.com/") {
                        parse_github_pr_url(&owner_or_url).ok_or_else(|| {
                            codereview_lib::error::AppError::Other(
                                format!("could not parse GitHub PR URL: {owner_or_url}"),
                            )
                        })?
                    } else {
                        let n = name.ok_or_else(|| {
                            codereview_lib::error::AppError::Other(
                                "name and number are required when not providing a URL".into(),
                            )
                        })?;
                        let num = number.ok_or_else(|| {
                            codereview_lib::error::AppError::Other(
                                "number is required when not providing a URL".into(),
                            )
                        })?;
                        (owner_or_url, n, num)
                    };
                let base_paths: Vec<String> = std::env::var("CODEREVIEW_BASE_PATHS")
                    .unwrap_or_default()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                let r = review::create_review_for_pr_impl(db, &owner, &repo_name, pr_number, &base_paths, false)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&r)?);
                } else {
                    println!("Review #{}", r.id);
                }
            }
        },

        Cmd::Update {
            review_id,
            body,
            verdict,
        } => {
            let conn = db.0.lock().unwrap();
            review::update_review_impl(&conn, review_id, body, verdict)?;
            if !json {
                println!("Updated review #{review_id}");
            }
        }

        Cmd::New { review_id } => {
            let target_id = {
                let conn = db.0.lock().unwrap();
                review::load_detail(&conn, review_id)?.review.target_id
            };
            let r = review::new_review_on_target_live(db, target_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&r)?);
            } else {
                println!("Review #{}", r.id);
            }
        }

        Cmd::Delete { review_id } => {
            let conn = db.0.lock().unwrap();
            review::delete_review_impl(&conn, review_id)?;
            if !json {
                println!("Deleted review #{review_id}");
            }
        }

        Cmd::Comment { action } => match action {
            CommentAction::Add {
                review_id,
                file,
                line,
                side,
                body,
                tag,
            } => {
                let conn = db.0.lock().unwrap();
                let c = review::add_comment_impl(
                    &conn,
                    review_id,
                    file,
                    side,
                    line,
                    None, // start_line
                    None, // diff_hunk
                    body,
                    None, // anchored_head_sha
                    None, // parent_id
                    parse_tag(&tag),
                )?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&c)?);
                } else {
                    println!("Added comment #{}", c.id);
                }
            }
            CommentAction::AddFile {
                review_id,
                file,
                body,
                tag,
            } => {
                let conn = db.0.lock().unwrap();
                let c = review::add_file_comment_impl(&conn, review_id, file, body, parse_tag(&tag))?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&c)?);
                } else {
                    println!("Added file comment #{}", c.id);
                }
            }
            CommentAction::Update { comment_id, body } => {
                let conn = db.0.lock().unwrap();
                review::update_comment_impl(&conn, comment_id, body)?;
                if !json {
                    println!("Updated comment #{comment_id}");
                }
            }
            CommentAction::Tag { comment_id, tag } => {
                let conn = db.0.lock().unwrap();
                review::set_comment_tag_impl(&conn, comment_id, parse_tag(&tag))?;
                if !json {
                    println!("Tagged comment #{comment_id} as {tag}");
                }
            }
            CommentAction::Delete { comment_id } => {
                let conn = db.0.lock().unwrap();
                review::delete_comment_impl(&conn, comment_id)?;
                if !json {
                    println!("Deleted comment #{comment_id}");
                }
            }
            CommentAction::Resolve { comment_id } => {
                let conn = db.0.lock().unwrap();
                review::set_resolved(&conn, comment_id, true)?;
                if !json {
                    println!("Resolved comment #{comment_id}");
                }
            }
            CommentAction::Unresolve { comment_id } => {
                let conn = db.0.lock().unwrap();
                review::set_resolved(&conn, comment_id, false)?;
                if !json {
                    println!("Unresolved comment #{comment_id}");
                }
            }
            CommentAction::Reply {
                review_id,
                parent_id,
                body,
                tag,
            } => {
                let conn = db.0.lock().unwrap();
                let c = review::add_comment_impl(
                    &conn,
                    review_id,
                    String::new(),
                    "RIGHT".to_string(),
                    0,
                    None,
                    None,
                    body,
                    None,
                    Some(parent_id),
                    parse_tag(&tag),
                )?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&c)?);
                } else {
                    println!("Added reply #{}", c.id);
                }
            }
            CommentAction::DraftReply {
                review_id,
                thread_comment_id,
                file,
                side,
                line,
                body,
            } => {
                let conn = db.0.lock().unwrap();
                let c = review::add_draft_reply_impl(
                    &conn,
                    review_id,
                    thread_comment_id,
                    body,
                    file,
                    side,
                    line,
                    None,
                )?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&c)?);
                } else {
                    println!("Drafted reply #{} to GitHub thread comment {thread_comment_id}", c.id);
                }
            }
            CommentAction::PublishReplies { review_id } => {
                let result = review::publish_draft_replies_impl(db, review_id)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else {
                    println!(
                        "Published {} draft repl{}, {} failed",
                        result.published,
                        if result.published == 1 { "y" } else { "ies" },
                        result.failed,
                    );
                    for err in &result.errors {
                        eprintln!("  error: {err}");
                    }
                }
            }
            CommentAction::PublishReply { comment_id } => {
                review::publish_draft_reply_impl(db, comment_id)?;
                if json {
                    println!("{}", serde_json::json!({ "published": true }));
                } else {
                    println!("Published draft reply #{comment_id}");
                }
            }
        },

        Cmd::Export {
            review_id,
            format,
            output,
        } => {
            let conn = db.0.lock().unwrap();
            match output {
                Some(path) => {
                    export::export_review_impl(&conn, review_id, &path, &format)?;
                    eprintln!("Exported review #{review_id} to {path}");
                }
                None => {
                    let content = export::render(&conn, review_id, &format)?;
                    print!("{content}");
                }
            }
        }

        Cmd::Repo { action } => match action {
            RepoAction::List => {
                let conn = db.0.lock().unwrap();
                let repos = repo::list_repositories_impl(&conn)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&repos)?);
                } else {
                    if repos.is_empty() {
                        println!("No repositories registered.");
                        return Ok(());
                    }
                    println!("{:<6} {:<20} {:<50}", "ID", "REMOTE", "LOCAL PATH");
                    for r in &repos {
                        let remote = match (&r.remote_owner, &r.remote_name) {
                            (Some(o), Some(n)) => format!("{o}/{n}"),
                            _ => "-".into(),
                        };
                        let path = r.local_path.as_deref().unwrap_or("(remote-only)");
                        println!("{:<6} {:<20} {:<50}", r.id, remote, path);
                    }
                }
            }
            RepoAction::Add { path } => {
                let conn = db.0.lock().unwrap();
                let r = repo::add_repository_impl(&conn, &path)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&r)?);
                } else {
                    println!("Added repository #{} ({})", r.id, r.local_path.as_deref().unwrap_or("remote-only"));
                }
            }
            RepoAction::Remove { id } => {
                let conn = db.0.lock().unwrap();
                repo::remove_repository_impl(&conn, id)?;
                if !json {
                    println!("Removed repository #{id}");
                }
            }
        },

        Cmd::Open { target } => {
            let url = match target {
                OpenTarget::Review { review_id } => format!("codereview://review/{review_id}"),
                OpenTarget::Pr { owner, name, number } => {
                    format!("codereview://pr/{owner}/{name}/{number}")
                }
                OpenTarget::Diff { repo_id, base_ref, head_ref, three_dot } => format!(
                    "codereview://diff?repo={repo_id}&base={base_ref}&head={head_ref}&three_dot={three_dot}"
                ),
                OpenTarget::Graph { path, ref_name, no_ref, repo } => {
                    let t = resolve_graph_target(db, path, ref_name, no_ref, repo, json)?;
                    let mut url = url::Url::parse(&format!("codereview://repo/{}", t.repo_id))
                        .map_err(|e| AppError::Other(format!("bad URL: {e}")))?;
                    {
                        let mut q = url.query_pairs_mut();
                        if let Some(r) = &t.ref_name {
                            q.append_pair("ref", r);
                        }
                        if let Some(w) = &t.worktree {
                            q.append_pair("worktree", w);
                        }
                    }
                    if url.query() == Some("") {
                        url.set_query(None);
                    }
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "url": url.as_str(),
                                "repo_id": t.repo_id,
                                "ref": t.ref_name,
                                "worktree": t.worktree,
                            })
                        );
                    }
                    url.to_string()
                }
                OpenTarget::Home { section } => match section {
                    Some(s) => format!("codereview://home/{s}"),
                    None => "codereview://home".to_string(),
                },
                OpenTarget::Url { url } => url,
            };
            open::that(&url).map_err(|e| {
                codereview_lib::error::AppError::Other(format!("failed to open URL: {e}"))
            })?;
            if !json {
                eprintln!("Opened: {url}");
            }
        }

        Cmd::Inbox { action } => match action {
            InboxAction::List => {
                let conn = db.0.lock().unwrap();
                let items = inbox::list_inbox_impl(&conn)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&items)?);
                } else {
                    if items.is_empty() {
                        println!("Inbox is empty.");
                        return Ok(());
                    }
                    println!(
                        "{:<40} {:<8} {:<30} {:<15} REASON",
                        "REPO", "NUMBER", "TITLE", "AUTHOR"
                    );
                    for item in &items {
                        let title = truncate(&item.item.title, 28);
                        let author = item
                            .item
                            .author_login
                            .as_deref()
                            .unwrap_or("-");
                        let reasons = item.reasons.iter().map(|r| r.reason.as_str()).collect::<Vec<_>>().join(",");
                        println!(
                            "{:<40} {:<8} {:<30} {:<15} {}{}",
                            truncate(&item.item.repo, 38),
                            item.item.number,
                            title,
                            author,
                            reasons,
                            if item.item.stale { " (stale)" } else { "" },
                        );
                    }
                }
            }
            InboxAction::Archive { reference } => set_inbox_archived(db, &reference, true)?,
            InboxAction::Forget { reference } => forget_inbox_item(db, &reference)?,
            InboxAction::Restore { reference } => set_inbox_archived(db, &reference, false)?,
            InboxAction::Sync => print_sync(&sync_once(db, Scope::Inbox, Mode::Forced)?, json)?,
        },

        Cmd::Focus { action } => run_focus(action, json, db)?,

        Cmd::Threads {
            owner,
            name,
            number,
        } => {
            let threads = provider_for().pr_review_threads(&owner, &name, number)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&threads)?);
            } else {
                if threads.is_empty() {
                    println!("No review threads found.");
                    return Ok(());
                }
                for t in &threads {
                    let loc = match (&t.path, t.line) {
                        (Some(p), Some(l)) => format!("{p}:{l}"),
                        (Some(p), None) => p.clone(),
                        _ => "(file-level)".to_string(),
                    };
                    let status = if t.is_resolved { "resolved" } else { "open" };
                    println!("Thread {} — {loc} [{status}]", t.id);
                    for c in &t.comments {
                        let login = c.author.as_ref().and_then(|a| a.login.as_deref()).unwrap_or("unknown");
                        let db_id = c.database_id.map(|n| n.to_string()).unwrap_or_else(|| "-".to_string());
                        println!("  [{db_id}] {login}: {}", c.body.lines().next().unwrap_or(""));
                    }
                    println!();
                }
            }
        }

        Cmd::Board { action } => run_board(action, json, db)?,
        Cmd::Sync { full, light } => {
            let mode = if full {
                Mode::Full
            } else if light {
                Mode::Light
            } else {
                Mode::Forced
            };
            print_sync(&sync_once(db, Scope::All, mode)?, json)?;
        }
    }
    Ok(())
}

/// The one-shot in-process sync (ADR 0003): no scheduler, no events; the app
/// sees the fresh rows on its next read.
fn sync_once(db: &Db, scope: Scope, mode: Mode) -> AppResult<SyncReport> {
    sync::run(db, &SyncRequest::new(scope, mode), &|_| {})
}

fn set_inbox_archived(db: &Db, reference: &str, archive: bool) -> AppResult<()> {
    let (owner, name, number) = board::parse_ref(reference)?;
    let conn = db.0.lock().unwrap();
    let id = inbox::find_item_id(&conn, &owner, &name, number)?
        .ok_or_else(|| AppError::Other(format!("{owner}/{name}#{number} isn't in the inbox")))?;
    if archive {
        inbox::archive_item_impl(&conn, &id)?;
    } else {
        inbox::restore_item_impl(&conn, &id)?;
    }
    println!("{owner}/{name}#{number} {}.", if archive { "archived" } else { "restored to New" });
    Ok(())
}

fn forget_inbox_item(db: &Db, reference: &str) -> AppResult<()> {
    let (owner, name, number) = board::parse_ref(reference)?;
    let conn = db.0.lock().unwrap();
    let id = inbox::find_item_id(&conn, &owner, &name, number)?
        .ok_or_else(|| AppError::Other(format!("{owner}/{name}#{number} isn't in the inbox")))?;
    inbox::forget_item_impl(&conn, &id)?;
    println!("{owner}/{name}#{number} archived and forgotten.");
    Ok(())
}

fn print_sync(r: &SyncReport, json: bool) -> AppResult<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(r)?);
    } else {
        println!(
            "Synced for {} — {} inbox items ({} closed), {} board cards ({} newly tracked); \
             {} probed, {} refetched, {} threads ({}ms)",
            r.viewer_login,
            r.item_count,
            r.closed_count,
            r.card_count,
            r.auto_tracked,
            r.probed,
            r.changed,
            r.threads,
            r.duration_ms,
        );
    }
    Ok(())
}

fn run_board(action: BoardAction, json: bool, db: &Db) -> AppResult<()> {
    match action {
        BoardAction::List { lane } => {
            let conn = db.0.lock().unwrap();
            let mut cards = board_cmd::board_list_impl(&conn)?;
            if let Some(lane) = &lane {
                cards.retain(|c| c.lane.as_deref() == Some(lane.as_str()));
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&cards)?);
            } else {
                if cards.is_empty() {
                    println!("No cards on the board.");
                    return Ok(());
                }
                println!("{:<10} {:<30} {:<8} {:<40}", "LANE", "REPO", "NUMBER", "TITLE");
                for c in &cards {
                    println!(
                        "{:<10} {:<30} {:<8} {:<40}",
                        c.lane.as_deref().unwrap_or("-"),
                        truncate(&c.card.repo, 28),
                        c.card.number,
                        truncate(&c.card.title, 38),
                    );
                }
            }
        }

        BoardAction::Show { reference, here } => {
            let reference = resolve_reference(reference, here)?;
            let (owner, name, number) = board::parse_ref(&reference)?;
            let conn = db.0.lock().unwrap();
            let card_id = board_cmd::find_card_id(&conn, &owner, &name, number)?
                .ok_or_else(|| AppError::Other(format!("{owner}/{name}#{number} isn't tracked on the board")))?;
            let card = board_cmd::board_show_impl(&conn, &card_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&card)?);
            } else {
                println!("{} {}#{} — {}", card.card.typ, card.card.repo, card.card.number, card.card.title);
                println!("Lane:  {}", card.lane.as_deref().unwrap_or("(hidden)"));
                println!("State: {}", card.card.state.as_deref().unwrap_or("-"));
                if card.card.typ == "pr" {
                    println!("CI:    {}", card.card.ci_status.as_deref().unwrap_or("-"));
                    println!("Merge: {}", card.card.mergeable.as_deref().unwrap_or("-"));
                    println!("Head:  {}", card.card.merge_state_status.as_deref().unwrap_or("-"));
                    if let Some(stack) = card.card.stack_number {
                        println!("Stack: #{stack} (position {})", card.card.stack_position.unwrap_or(0));
                    }
                }
                println!("URL:   {}", card.card.url);
                if !card.meta.is_empty() {
                    println!("\nMetadata:");
                    for m in &card.meta {
                        let tag = m.tag.as_deref().map(|t| format!(" [{t}]")).unwrap_or_default();
                        println!("  [#{}] {}{tag}: {}", m.id, m.kind, m.value);
                    }
                }
            }
        }

        BoardAction::Track { reference } => {
            let card_id = board_cmd::board_track_impl(db, &reference)?;
            if json {
                println!("{}", serde_json::json!({ "cardId": card_id }));
            } else {
                println!("Tracked {reference} (card {card_id})");
            }
        }

        BoardAction::Sync => print_sync(&sync_once(db, Scope::Board, Mode::Forced)?, json)?,

        BoardAction::Link { issue_id, pr_reference } => {
            board_cmd::board_link_pr_impl(db, &issue_id, &pr_reference)?;
            if !json {
                println!("Linked {pr_reference} to card {issue_id}");
            }
        }

        BoardAction::Meta { action } => run_board_meta(action, json, db)?,
    }
    Ok(())
}

fn run_board_meta(action: BoardMetaAction, json: bool, db: &Db) -> AppResult<()> {
    match action {
        BoardMetaAction::Add {
            reference,
            here,
            kind,
            value,
            tag,
            label,
            cwd,
            track,
        } => {
            let reference = resolve_reference(reference, here)?;
            let card_id = board_cmd::resolve_or_track_card_id(db, &reference, track)?;
            let conn = db.0.lock().unwrap();
            let m = board_cmd::board_meta_add_impl(
                &conn,
                &card_id,
                &kind,
                &value,
                tag.as_deref(),
                label.as_deref(),
                cwd.as_deref(),
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&m)?);
            } else {
                println!("Added {} metadata #{} to card {card_id}", m.kind, m.id);
            }
        }

        BoardMetaAction::List { reference, here } => {
            let reference = resolve_reference(reference, here)?;
            let (owner, name, number) = board::parse_ref(&reference)?;
            let conn = db.0.lock().unwrap();
            let card_id = board_cmd::find_card_id(&conn, &owner, &name, number)?
                .ok_or_else(|| AppError::Other(format!("{owner}/{name}#{number} isn't tracked on the board")))?;
            let items = board_cmd::board_meta_list_impl(&conn, &card_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&items)?);
            } else {
                if items.is_empty() {
                    println!("No metadata on this card.");
                    return Ok(());
                }
                for m in &items {
                    let tag = m.tag.as_deref().map(|t| format!(" [{t}]")).unwrap_or_default();
                    println!("[#{}] {}{tag}: {}", m.id, m.kind, m.value);
                }
            }
        }

        BoardMetaAction::Rm { id } => {
            let conn = db.0.lock().unwrap();
            board_cmd::board_meta_delete_impl(&conn, id)?;
            if !json {
                println!("Deleted metadata #{id}");
            }
        }
    }
    Ok(())
}

fn run_focus(action: FocusAction, json: bool, db: &Db) -> AppResult<()> {
    let conn = db.0.lock().unwrap();
    match action {
        FocusAction::Hunks { review_id } => {
            let hunks = focus::focus_hunks_impl(&conn, review_id)?;
            if hunks.len() > FOCUS_HUNK_WARN {
                eprintln!(
                    "warning: {} hunks — consider tiering by file and fetching only what you need",
                    hunks.len()
                );
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&hunks)?);
                return Ok(());
            }
            for h in &hunks {
                let rule = match (&h.rule_tier, &h.rule_reason) {
                    (Some(t), Some(r)) => format!("  [{t}: {r}]"),
                    _ => String::new(),
                };
                let header = if h.header.is_empty() { "(no hunks)" } else { &h.header };
                println!(
                    "{}  {}  {}  +{} -{}{rule}",
                    h.hunk_id, h.file_path, header, h.additions, h.deletions
                );
                for l in &h.preview {
                    println!("    {l}");
                }
            }
        }
        FocusAction::Hunk { review_id, hunk_id } => {
            println!("{}", focus::focus_hunk_text_impl(&conn, review_id, &hunk_id)?);
        }
        FocusAction::Set {
            review_id,
            file,
            level,
        } => {
            let raw = if file == "-" {
                std::io::read_to_string(std::io::stdin())?
            } else {
                std::fs::read_to_string(&file)?
            };
            let plan: focus::Plan = serde_json::from_str(&raw)?;
            let view = focus::set_focus_plan_impl(&conn, review_id, plan, level.as_deref())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&view)?);
            } else {
                println!(
                    "Focus set on review #{review_id}: {} hunks, level {}",
                    view.hunks.len(),
                    view.focus.level
                );
                print_tier_counts(&view.hunks);
            }
        }
        FocusAction::Show { review_id } => {
            let view = focus::get_focus_impl(&conn, review_id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&view)?);
                return Ok(());
            }
            let Some(view) = view else {
                println!("Review #{review_id} has no focus.");
                return Ok(());
            };
            let f = &view.focus;
            println!("Focus on review #{review_id}");
            println!("Status: {}", f.status);
            if let Some(e) = &f.error {
                println!("Error:  {e}");
            }
            println!("Level:  {}", f.level);
            if let Some(m) = &f.model {
                println!("Model:  {m}");
            }
            println!(
                "Head:   {}{}",
                f.head_sha.as_deref().unwrap_or("-"),
                if view.stale { " (stale: head or base moved)" } else { "" }
            );
            if let Some(s) = &f.summary {
                println!("\n## Summary\n{s}");
            }
            println!();
            print_tier_counts(&view.hunks);
            let notes = focus::focus_comments_impl(&conn, review_id)?;
            for c in &notes {
                println!("\n  [#{}] {}:{} ({})\n        {}", c.id, c.file_path, c.line, c.side, c.body);
            }
        }
        FocusAction::Clear { review_id } => {
            focus::clear_focus_impl(&conn, review_id)?;
            println!("Cleared focus on review #{review_id}");
        }
    }
    Ok(())
}

fn print_tier_counts(hunks: &[focus::FocusEntry]) {
    let counts: Vec<String> = focus::TIERS
        .iter()
        .rev()
        .map(|t| format!("{t}: {}", hunks.iter().filter(|h| h.tier == *t).count()))
        .collect();
    println!("{}", counts.join("  "));
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}...", &s[..max.saturating_sub(3)])
    } else {
        s.to_string()
    }
}
