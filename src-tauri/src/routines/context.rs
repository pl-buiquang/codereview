//! Turn a launch reference (a board card, a review, the board) into the canonical context key used
//! for dedup, and — when a run starts — into prompt variables, a context block and a working directory.

use std::path::PathBuf;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use super::{manager, Routine};
use crate::commands::{board, chat, review};
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::worker::request::{WorkspaceKind, WorkspaceSpec};

/// What a menu launches a routine on. `id` is the board card's node id for `pr`/`issue`, the review
/// id for `review`, and empty for `board`. `base_paths` is only needed to create the review for a
/// PR card that has none yet (same as the board's "Open in terminal").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextRef {
    pub kind: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub base_paths: Vec<String>,
}

/// Cheap, network-free description of a context: enough to dedup and to list eligible routines.
#[derive(Debug, Clone)]
pub struct Light {
    /// Canonical key: `pr:owner/name#12`, `issue:owner/name#3`, `review:7`, `board`. A review of a
    /// GitHub PR gets the PR's key, so launching from the card or the review dedups.
    pub key: String,
    pub label: String,
    /// Routine context types this context satisfies.
    pub kinds: Vec<&'static str>,
}

pub fn pr_key(owner: &str, name: &str, number: i64) -> String {
    format!("pr:{}/{}#{number}", owner.to_lowercase(), name.to_lowercase())
}

pub fn light(db: &Db, r: &ContextRef) -> AppResult<Light> {
    let conn = db.0.lock().unwrap();
    match r.kind.as_str() {
        "board" => Ok(Light { key: "board".into(), label: "Board".into(), kinds: vec!["board"] }),
        "pr" | "issue" => {
            let card = board::card_row(&conn, &r.id)?;
            if card.typ != r.kind {
                return Err(AppError::Other(format!("card is not a {}", r.kind)));
            }
            let kind: &'static str = if r.kind == "pr" { "pr" } else { "issue" };
            let key = match card.repo.split_once('/') {
                Some((o, n)) if kind == "pr" => pr_key(o, n, card.number),
                _ => format!("{kind}:{}#{}", card.repo.to_lowercase(), card.number),
            };
            Ok(Light { key, label: format!("{}#{}", card.repo, card.number), kinds: vec![kind] })
        }
        "review" => {
            let review_id: i64 = r.id.parse().map_err(|_| AppError::Other("bad review id".into()))?;
            let detail = review::load_detail(&conn, review_id)?;
            match (&detail.remote_owner, &detail.remote_name, detail.target.github_pr_number) {
                (Some(o), Some(n), Some(num)) if detail.target.kind == "github_pr" => Ok(Light {
                    key: pr_key(o, n, num),
                    label: format!("{o}/{n}#{num}"),
                    kinds: vec!["review", "pr"],
                }),
                _ => Ok(Light {
                    key: format!("review:{review_id}"),
                    label: detail.target.title.clone(),
                    kinds: vec!["review"],
                }),
            }
        }
        other => Err(AppError::Other(format!("unknown context kind `{other}`"))),
    }
}

/// A routine is offered on a context when its type is one the context satisfies.
pub fn eligible(routine: &Routine, light: &Light) -> bool {
    routine.enabled && light.kinds.contains(&routine.context_type.as_str())
}

/// Everything a run needs once it starts. Resolving may hit the network and create a worktree, so
/// it runs on the run's worker thread, never in the launching command.
pub struct Resolved {
    pub vars: Vec<(String, String)>,
    pub cwd: PathBuf,
    pub block: String,
}

#[derive(Default)]
struct Facts {
    repo: String,
    number: String,
    url: String,
    title: String,
    head_ref: String,
    base_ref: String,
    review_id: Option<i64>,
    kind: &'static str,
    /// Local clone of the repo, when known, used as the cwd of issue routines that want a checkout.
    local_path: Option<String>,
    /// `owner/name` on GitHub, when the context has one (a remote host needs it to find its clone).
    github_repo: Option<String>,
    /// The board card the context is, or stands for.
    card_id: Option<String>,
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("codereview_routine_{tag}"));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn pr_refs(conn: &Connection, node_id: &str) -> (String, String) {
    conn.query_row("SELECT head_ref, base_ref FROM gh_pr WHERE node_id = ?1", params![node_id], |r| {
        Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get::<_, Option<String>>(1)?.unwrap_or_default()))
    })
    .unwrap_or_default()
}

pub(crate) fn repo_local_path(conn: &Connection, repo: &str) -> Option<String> {
    let (owner, name) = repo.split_once('/')?;
    conn.query_row(
        "SELECT local_path FROM repository WHERE LOWER(remote_owner) = LOWER(?1) AND LOWER(remote_name) = LOWER(?2)",
        params![owner, name],
        |r| r.get::<_, Option<String>>(0),
    )
    .ok()
    .flatten()
}

/// What the context says about itself. `local` also prepares what a run on this machine needs (a
/// review of the PR to hang a worktree on); a remote run does not want that.
fn gather(app: &AppHandle, routine: &Routine, r: &ContextRef, local: bool) -> AppResult<Facts> {
    let db = app.state::<Db>();
    let mut facts = Facts { kind: "board", ..Facts::default() };
    match r.kind.as_str() {
        "board" => {}
        "pr" => {
            let card = board::card_row(&db.0.lock().unwrap(), &r.id)?;
            let (head_ref, base_ref) = pr_refs(&db.0.lock().unwrap(), &r.id);
            facts = Facts {
                repo: card.repo.clone(),
                number: card.number.to_string(),
                url: card.url.clone(),
                title: card.title.clone(),
                head_ref: card.head_ref.clone().filter(|h| !h.is_empty()).unwrap_or(head_ref),
                base_ref,
                kind: "pr",
                github_repo: Some(card.repo.clone()),
                card_id: Some(card.id.clone()),
                ..Facts::default()
            };
            if routine.needs_worktree && local {
                let (owner, name, number) = board::pr_card_coords(&db, &r.id)?;
                let rev = review::create_review_for_pr_impl(&db, &owner, &name, number, &r.base_paths, true)?;
                facts.review_id = Some(rev.id);
            }
        }
        "issue" => {
            let card = board::card_row(&db.0.lock().unwrap(), &r.id)?;
            facts = Facts {
                repo: card.repo.clone(),
                number: card.number.to_string(),
                url: card.url.clone(),
                title: card.title.clone(),
                kind: "issue",
                local_path: repo_local_path(&db.0.lock().unwrap(), &card.repo),
                github_repo: Some(card.repo.clone()),
                card_id: Some(card.id.clone()),
                ..Facts::default()
            };
        }
        "review" => {
            let review_id: i64 = r.id.parse().map_err(|_| AppError::Other("bad review id".into()))?;
            let detail = review::load_detail(&db.0.lock().unwrap(), review_id)?;
            let (repo, url) = match (&detail.remote_owner, &detail.remote_name, detail.target.github_pr_number) {
                (Some(o), Some(n), Some(num)) => (format!("{o}/{n}"), format!("https://github.com/{o}/{n}/pull/{num}")),
                _ => (detail.local_path.clone().unwrap_or_default(), String::new()),
            };
            let github_repo = (detail.target.kind == "github_pr" && !url.is_empty()).then(|| repo.clone());
            facts = Facts {
                repo,
                number: detail.target.github_pr_number.map(|n| n.to_string()).unwrap_or_default(),
                url,
                title: detail.target.title.clone(),
                head_ref: detail.target.head_ref.clone(),
                base_ref: detail.target.base_ref.clone(),
                review_id: Some(review_id),
                kind: "review",
                local_path: detail.local_path.clone(),
                github_repo,
                card_id: None,
            };
        }
        other => return Err(AppError::Other(format!("unknown context kind `{other}`"))),
    }
    Ok(facts)
}

pub fn resolve(app: &AppHandle, routine: &Routine, r: &ContextRef) -> AppResult<Resolved> {
    let db = app.state::<Db>();
    let facts = gather(app, routine, r, true)?;

    let cwd = if routine.needs_worktree {
        match facts.review_id {
            Some(id) => {
                let path = tauri::async_runtime::block_on(chat::ensure_chat_worktree(id, app.state::<Db>()))?;
                PathBuf::from(path)
            }
            None => match &facts.local_path {
                Some(p) if std::path::Path::new(p).is_dir() => PathBuf::from(p),
                _ => {
                    return Err(AppError::Other(
                        "this routine needs a checkout, but the context has no local clone — link one in Repositories".into(),
                    ))
                }
            },
        }
    } else {
        scratch_dir(&format!("{}_{}", routine.key, facts.kind))
    };

    let worktree = if routine.needs_worktree { cwd.to_string_lossy().into_owned() } else { String::new() };
    let plans_dir = crate::commands::repo::expand_home(&manager::routine_settings(&db.0.lock().unwrap()).plans_dir);
    let vars = vec![
        ("plans_dir".to_string(), plans_dir),
        ("repo".to_string(), facts.repo.clone()),
        ("number".to_string(), facts.number.clone()),
        ("url".to_string(), facts.url.clone()),
        ("title".to_string(), facts.title.clone()),
        ("head_ref".to_string(), facts.head_ref.clone()),
        ("base_ref".to_string(), facts.base_ref.clone()),
        ("worktree".to_string(), worktree),
        ("review_id".to_string(), facts.review_id.map(|i| i.to_string()).unwrap_or_default()),
    ];
    let block = context_block(routine, &facts, &cwd.to_string_lossy());
    Ok(Resolved { vars, cwd, block })
}

/// A context resolved for a run on a remote host: the prompt variables and context block, where the
/// host should run it, and the snapshot that answers `cr` reads there.
pub struct RemoteResolved {
    pub vars: Vec<(String, String)>,
    pub block: String,
    pub workspace: WorkspaceSpec,
    pub snapshot: serde_json::Value,
}

/// The workspace a remote host should prepare for a run, or why it can't be named.
fn workspace_for(routine: &Routine, f: &Facts) -> AppResult<WorkspaceSpec> {
    let repo = f.github_repo.clone().filter(|r| r.split_once('/').is_some_and(|(o, n)| !o.is_empty() && !n.is_empty() && !n.contains('/')));
    let Some(repo) = repo else {
        return Err(AppError::Other(format!(
            "“{}” can't run on a remote host from this context: the host needs a GitHub repository (owner/name), and this context has none",
            routine.name
        )));
    };
    let number: Option<i64> = f.number.parse().ok();
    Ok(match (f.kind, number) {
        ("pr" | "review", Some(pr)) if routine.needs_worktree => WorkspaceSpec {
            repo,
            pr: Some(pr),
            branch: Some(f.head_ref.clone()).filter(|b| !b.is_empty()),
            kind: WorkspaceKind::PrWorktree,
        },
        _ => WorkspaceSpec { repo, pr: None, branch: None, kind: WorkspaceKind::MainClone },
    })
}

/// Everything a remote run needs from its context. Nothing here touches the disk: the host owns its
/// clones, and the request names `owner/name` + PR, never a local path.
pub fn resolve_remote(app: &AppHandle, routine: &Routine, r: &ContextRef) -> AppResult<RemoteResolved> {
    let db = app.state::<Db>();
    let facts = gather(app, routine, r, false)?;
    let workspace = workspace_for(routine, &facts)?;
    let plans_dir = crate::commands::repo::expand_home(&manager::routine_settings(&db.0.lock().unwrap()).plans_dir);
    let here = "the current directory (a checkout on the remote host)";
    let vars = vec![
        ("plans_dir".to_string(), plans_dir),
        ("repo".to_string(), facts.repo.clone()),
        ("number".to_string(), facts.number.clone()),
        ("url".to_string(), facts.url.clone()),
        ("title".to_string(), facts.title.clone()),
        ("head_ref".to_string(), facts.head_ref.clone()),
        ("base_ref".to_string(), facts.base_ref.clone()),
        ("worktree".to_string(), if routine.needs_worktree { here.to_string() } else { String::new() }),
        ("review_id".to_string(), facts.review_id.map(|i| i.to_string()).unwrap_or_default()),
    ];
    let block = context_block(routine, &facts, here);
    let snapshot = snapshot(&db.0.lock().unwrap(), &facts, &workspace);
    Ok(RemoteResolved { vars, block, workspace, snapshot })
}

/// The context snapshot a remote run reads with `cr board ...` / `cr focus show`: the context's card
/// and the cards linked to it (as `cr board show --json` prints them), and the focus of its review.
fn snapshot(conn: &Connection, f: &Facts, ws: &WorkspaceSpec) -> serde_json::Value {
    let mut cards: Vec<serde_json::Value> = Vec::new();
    if let Ok(all) = board::board_list_impl(conn) {
        let own = f.card_id.clone().or_else(|| {
            let number = ws.pr.or_else(|| f.number.parse().ok())?;
            let (owner, name) = ws.repo.split_once('/')?;
            board::find_card_id(conn, owner, name, number).ok().flatten()
        });
        if let Some(own) = own {
            let linked: Vec<String> = all
                .iter()
                .find(|c| c.card.id == own)
                .map(|c| c.linked_issue_ids.iter().chain(&c.linked_pr_ids).cloned().collect())
                .unwrap_or_default();
            cards = all
                .iter()
                .filter(|c| c.card.id == own || linked.contains(&c.card.id))
                .filter_map(|c| serde_json::to_value(c).ok())
                .collect();
        }
    }
    let mut focus = serde_json::Map::new();
    if let Some(id) = f.review_id {
        if let Ok(view) = crate::commands::focus::get_focus_impl(conn, id) {
            focus.insert(id.to_string(), serde_json::to_value(view).unwrap_or(serde_json::Value::Null));
        }
    }
    serde_json::json!({ "cards": cards, "focus": focus })
}

fn context_block(routine: &Routine, f: &Facts, cwd: &str) -> String {
    let mut lines = vec![
        format!("## Routine context ({})", routine.name),
        "You are running unattended as a routine launched from the CodeReview app. Nobody can answer questions: never use \
         AskUserQuestion, pick sensible defaults."
            .to_string(),
    ];
    let mut fact = |label: &str, value: &str| {
        if !value.is_empty() {
            lines.push(format!("- {label}: {value}"));
        }
    };
    fact("Context", f.kind);
    fact("Repository", &f.repo);
    fact("Number", &f.number);
    fact("URL", &f.url);
    fact("Title", &f.title);
    fact("Head branch", &f.head_ref);
    fact("Base branch", &f.base_ref);
    fact("Review id", &f.review_id.map(|i| i.to_string()).unwrap_or_default());
    fact("Working directory", cwd);
    lines.push(
        "If you cannot finish without a human, end your final message with a line `NEEDS_ATTENTION: <one-sentence \
         diagnosis>`."
            .to_string(),
    );
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routine(context_type: &str, enabled: bool) -> Routine {
        Routine {
            id: 1,
            key: "k".into(),
            name: "K".into(),
            description: String::new(),
            context_type: context_type.into(),
            prompt_template: String::new(),
            system_prompt: String::new(),
            kind: "plain".into(),
            needs_worktree: false,
            access: "full".into(),
            harness_overrides: serde_json::json!({}),
            profile_id: None,
            model: String::new(),
            timeout_min: 1,
            builtin: false,
            handler: None,
            enabled,
        }
    }

    #[test]
    fn review_of_a_pr_satisfies_pr_routines_but_not_the_other_way() {
        let review_on_pr = Light { key: "pr:o/n#1".into(), label: String::new(), kinds: vec!["review", "pr"] };
        let pr_card = Light { key: "pr:o/n#1".into(), label: String::new(), kinds: vec!["pr"] };
        assert!(eligible(&routine("pr", true), &review_on_pr));
        assert!(eligible(&routine("review", true), &review_on_pr));
        assert!(!eligible(&routine("review", true), &pr_card));
        assert!(!eligible(&routine("pr", false), &pr_card));
    }

    fn facts(kind: &'static str, repo: Option<&str>, number: &str, head: &str) -> Facts {
        Facts {
            kind,
            github_repo: repo.map(str::to_string),
            number: number.into(),
            head_ref: head.into(),
            ..Facts::default()
        }
    }

    #[test]
    fn a_remote_workspace_names_the_repo_and_pr_never_a_path() {
        let mut r = routine("pr", true);
        let ws = workspace_for(&r, &facts("pr", Some("acme/widgets"), "7", "feat/x")).unwrap();
        assert_eq!((ws.repo.as_str(), ws.pr, ws.kind), ("acme/widgets", None, WorkspaceKind::MainClone));
        r.needs_worktree = true;
        let ws = workspace_for(&r, &facts("pr", Some("acme/widgets"), "7", "feat/x")).unwrap();
        assert_eq!((ws.pr, ws.branch.as_deref(), ws.kind), (Some(7), Some("feat/x"), WorkspaceKind::PrWorktree));
        // a review of a PR is the same; an issue has no PR to check out
        assert_eq!(workspace_for(&r, &facts("review", Some("acme/widgets"), "7", "")).unwrap().branch, None);
        assert_eq!(workspace_for(&r, &facts("issue", Some("acme/widgets"), "3", "")).unwrap().kind, WorkspaceKind::MainClone);
    }

    #[test]
    fn a_context_without_a_github_repo_is_refused_for_a_remote_run() {
        let r = routine("board", true);
        for f in [facts("board", None, "", ""), facts("review", None, "", ""), facts("pr", Some("/home/me/code/widgets"), "1", ""), facts("pr", Some("a/b/c"), "1", "")] {
            let err = workspace_for(&r, &f).unwrap_err().to_string();
            assert!(err.contains("can't run on a remote host"), "{err}");
        }
    }

    #[test]
    fn pr_key_is_case_insensitive() {
        assert_eq!(pr_key("Cardiologs", "Back", 12), pr_key("cardiologs", "back", 12));
    }
}
