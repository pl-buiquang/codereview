use std::process;
use std::sync::Mutex;

use clap::{Parser, Subcommand};

use codereview_lib::commands::{export, inbox, repo, review};
use codereview_lib::db::{self, Db};
use codereview_lib::db_path::resolve_db_path;
use codereview_lib::error::AppResult;
use codereview_lib::tools;

#[derive(Parser)]
#[command(name = "cr", about = "CodeReview CLI — browse and manage PR reviews")]
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
    /// Create a new review
    Create {
        #[command(subcommand)]
        target: CreateTarget,
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
}

#[derive(Subcommand)]
enum CreateTarget {
    /// Create a review for a local diff between two refs
    Local {
        repo_id: i64,
        base_ref: String,
        head_ref: String,
        /// Use three-dot diff (merge-base semantics, like GitHub PRs)
        #[arg(long)]
        three_dot: bool,
    },
    /// Create a review for a GitHub PR
    Pr {
        owner: String,
        name: String,
        number: i64,
    },
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
    },
    /// Update a comment's body
    Update {
        comment_id: i64,
        /// New body text
        #[arg(long)]
        body: String,
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
    /// List inbox items
    List,
    /// Refresh inbox from GitHub
    Refresh,
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
                    .filter(|c| c.parent_id.is_none())
                    .collect();
                if !root_comments.is_empty() {
                    println!("\n## Comments ({} total)\n", root_comments.len());
                    for c in root_comments {
                        let loc = if c.subject_type == "file" {
                            c.file_path.clone()
                        } else {
                            format!("{}:{} ({})", c.file_path, c.line, c.side)
                        };
                        println!("  [#{}] {}", c.id, loc);
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
            }
        }

        Cmd::Diff { review_id } => {
            let conn = db.0.lock().unwrap();
            let diff = review::review_diff_impl(&conn, review_id)?;
            print!("{diff}");
        }

        Cmd::Create { target } => match target {
            CreateTarget::Local {
                repo_id,
                base_ref,
                head_ref,
                three_dot,
            } => {
                let conn = db.0.lock().unwrap();
                let repos = repo::list_repositories_impl(&conn)?;
                let repo_row = repos.iter().find(|r| r.id == repo_id).ok_or_else(|| {
                    codereview_lib::error::AppError::Other(format!(
                        "repository {repo_id} not found"
                    ))
                })?;
                let r = review::create_review_impl(
                    &conn,
                    repo_id,
                    &repo_row.path,
                    &base_ref,
                    &head_ref,
                    three_dot,
                )?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&r)?);
                } else {
                    println!("Created review #{}", r.id);
                }
            }
            CreateTarget::Pr {
                owner,
                name,
                number,
            } => {
                let r = review::create_review_for_pr_impl(db, &owner, &name, number)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&r)?);
                } else {
                    println!("Created review #{}", r.id);
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
            } => {
                let conn = db.0.lock().unwrap();
                let c = review::add_file_comment_impl(&conn, review_id, file, body)?;
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
                    println!("{:<6} {:<20} {:<50}", "ID", "REMOTE", "PATH");
                    for r in &repos {
                        let remote = match (&r.remote_owner, &r.remote_name) {
                            (Some(o), Some(n)) => format!("{o}/{n}"),
                            _ => "-".into(),
                        };
                        println!("{:<6} {:<20} {:<50}", r.id, remote, r.path);
                    }
                }
            }
            RepoAction::Add { path } => {
                let conn = db.0.lock().unwrap();
                let r = repo::add_repository_impl(&conn, &path)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&r)?);
                } else {
                    println!("Added repository #{} ({})", r.id, r.path);
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
                        "{:<40} {:<8} {:<30} {:<15}",
                        "REPO", "NUMBER", "TITLE", "AUTHOR"
                    );
                    for item in &items {
                        let title = truncate(&item.item.title, 28);
                        let author = item
                            .item
                            .author_login
                            .as_deref()
                            .unwrap_or("-");
                        println!(
                            "{:<40} {:<8} {:<30} {:<15}",
                            truncate(&item.item.repo, 38),
                            item.item.number,
                            title,
                            author,
                        );
                    }
                }
            }
            InboxAction::Refresh => {
                let result = inbox::refresh_inbox_impl(db)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else {
                    println!(
                        "Refreshed inbox for {} — {} items, {} closed ({}ms)",
                        result.viewer_login,
                        result.item_count,
                        result.closed_count,
                        result.duration_ms,
                    );
                }
            }
        },
    }
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}...", &s[..max.saturating_sub(3)])
    } else {
        s.to_string()
    }
}
