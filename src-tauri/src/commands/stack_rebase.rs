//! Board "Rebase stack" button: preview what a local rebase would do, then do it.
//! The git work lives in [`crate::stack_rebase`]; this module maps a card to its
//! stack (from the mirror) and keeps the mirror fresh around it.

use std::path::PathBuf;
use std::time::Duration;

use rusqlite::params;
use serde::Serialize;
use tauri::State;

use crate::commands::board::refetch_card;
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::repo_sync;
use crate::stack_rebase::{self, ApplyReport, LocalCopy, Plan};

#[derive(Debug, Clone)]
struct StackEntry {
    number: i64,
    position: i64,
    state: String,
    head_ref: String,
    base_ref: String,
    can_update: bool,
}

struct StackCtx {
    repo_id: i64,
    owner: String,
    name: String,
    path: PathBuf,
    default_branch: Option<String>,
    entries: Vec<StackEntry>,
}

#[derive(Debug, Serialize)]
pub struct PreviewBranch {
    pub number: i64,
    pub branch: String,
    pub commits: u32,
    pub changed: bool,
    pub local: LocalCopy,
}

#[derive(Debug, Serialize)]
pub struct StackRebasePreview {
    pub ok: bool,
    /// Why the stack can't be rebased locally (when `!ok`).
    pub reasons: Vec<String>,
    /// Already on top of the trunk: nothing to do.
    pub noop: bool,
    pub trunk: Option<String>,
    pub branches: Vec<PreviewBranch>,
}

#[derive(Debug, Serialize)]
pub struct StackRebaseResult {
    pub report: ApplyReport,
}

fn load_stack(db: &Db, card_id: &str) -> AppResult<StackCtx> {
    let conn = db.0.lock().unwrap();
    let (repo_id, stack_number): (i64, Option<i64>) = conn
        .query_row(
            "SELECT p.repo_id, s.stack_number FROM gh_pr p
             LEFT JOIN gh_pr_status s ON s.pr_id = p.node_id WHERE p.node_id = ?1",
            params![card_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| AppError::Other("this card isn't a synced PR".into()))?;
    let stack_number = stack_number.ok_or_else(|| AppError::Other("this PR isn't part of a stack".into()))?;
    let (owner, name, path, default_branch): (Option<String>, Option<String>, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT remote_owner, remote_name, local_path, default_branch FROM repository WHERE id = ?1",
            params![repo_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
    let path = path.ok_or_else(|| AppError::Other("rebasing a stack needs a local clone of the repo".into()))?;
    let mut stmt = conn.prepare(
        "SELECT p.number, s.stack_position, COALESCE(p.state, ''), COALESCE(p.head_ref, ''),
                COALESCE(p.base_ref, ''), COALESCE(s.viewer_can_update_branch, 0)
         FROM gh_pr p JOIN gh_pr_status s ON s.pr_id = p.node_id
         WHERE p.repo_id = ?1 AND s.stack_number = ?2 ORDER BY s.stack_position",
    )?;
    let entries = stmt
        .query_map(params![repo_id, stack_number], |r| {
            Ok(StackEntry {
                number: r.get(0)?,
                position: r.get(1)?,
                state: r.get(2)?,
                head_ref: r.get(3)?,
                base_ref: r.get(4)?,
                can_update: r.get::<_, i64>(5)? != 0,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(StackCtx {
        repo_id,
        owner: owner.unwrap_or_default(),
        name: name.unwrap_or_default(),
        path: PathBuf::from(path),
        default_branch,
        entries,
    })
}

/// The trunk and the branches bottom-first, or every reason the stack's shape
/// rules out a local rebase.
fn shape(entries: &[StackEntry], default_branch: Option<&str>) -> Result<(String, Vec<String>), Vec<String>> {
    let mut reasons = Vec::new();
    let Some(bottom) = entries.first() else {
        return Err(vec!["the stack isn't synced yet — sync the board".into()]);
    };
    for e in entries {
        if !e.state.eq_ignore_ascii_case("open") {
            reasons.push(format!("#{} is {} — rebase on GitHub", e.number, e.state.to_lowercase()));
        }
        if !e.can_update {
            reasons.push(format!("you can't push to #{}'s branch", e.number));
        }
    }
    for w in entries.windows(2) {
        if w[1].position != w[0].position + 1 {
            reasons.push("some PRs of the stack aren't synced — sync the board".into());
            break;
        }
    }
    for w in entries.windows(2) {
        if w[1].base_ref != w[0].head_ref {
            reasons.push(format!("#{} isn't based on #{}'s branch", w[1].number, w[0].number));
        }
    }
    if let Some(def) = default_branch {
        if bottom.base_ref != def {
            reasons.push(format!("#{} targets {}, not {def}", bottom.number, bottom.base_ref));
        }
    }
    if reasons.is_empty() {
        Ok((bottom.base_ref.clone(), entries.iter().map(|e| e.head_ref.clone()).collect()))
    } else {
        Err(reasons)
    }
}

fn preview_of(ctx: &StackCtx, plan: Result<Plan, Vec<String>>, trunk: Option<String>) -> StackRebasePreview {
    match plan {
        Ok(plan) => StackRebasePreview {
            ok: true,
            reasons: vec![],
            noop: plan.is_noop(),
            trunk: Some(plan.trunk.clone()),
            branches: plan
                .branches
                .iter()
                .zip(&ctx.entries)
                .map(|(b, e)| PreviewBranch {
                    number: e.number,
                    branch: b.branch.clone(),
                    commits: b.commits,
                    changed: b.changed(),
                    local: b.local.clone(),
                })
                .collect(),
        },
        Err(reasons) => StackRebasePreview { ok: false, reasons, noop: false, trunk, branches: vec![] },
    }
}

fn compute(ctx: &StackCtx) -> (Result<Plan, Vec<String>>, Option<String>) {
    match shape(&ctx.entries, ctx.default_branch.as_deref()) {
        Ok((trunk, branches)) => (stack_rebase::plan(&ctx.path, &trunk, &branches), Some(trunk)),
        Err(reasons) => (Err(reasons), None),
    }
}

/// Dry run: fetches, then reports whether (and how) the stack would be rebased.
/// Changes nothing.
#[tauri::command]
pub async fn board_stack_rebase_preview(card_id: String, db: State<'_, Db>) -> AppResult<StackRebasePreview> {
    let ctx = load_stack(&db, &card_id)?;
    repo_sync::fetch_repo(&db, ctx.repo_id, Duration::ZERO)?;
    let (plan, trunk) = compute(&ctx);
    Ok(preview_of(&ctx, plan, trunk))
}

/// Rebases the whole stack onto its trunk, pushes it atomically and moves the
/// local branches. Re-plans from fresh GitHub + git state first, and refuses
/// (changing nothing) if anything no longer qualifies.
#[tauri::command]
pub async fn board_stack_rebase(card_id: String, db: State<'_, Db>) -> AppResult<StackRebaseResult> {
    let ctx = load_stack(&db, &card_id)?;
    for e in &ctx.entries {
        refetch_card(&db, &ctx.owner, &ctx.name, e.number)?;
    }
    repo_sync::fetch_repo(&db, ctx.repo_id, Duration::ZERO)?;
    let ctx = load_stack(&db, &card_id)?;
    let (plan, _) = compute(&ctx);
    let plan = plan.map_err(|r| stack_rebase::refusal_error(&r))?;
    let report = stack_rebase::apply(&ctx.path, &plan)?;
    repo_sync::fetch_repo(&db, ctx.repo_id, Duration::ZERO)?;
    for e in &ctx.entries {
        refetch_card(&db, &ctx.owner, &ctx.name, e.number)?;
    }
    Ok(StackRebaseResult { report })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(number: i64, position: i64, head: &str, base: &str) -> StackEntry {
        StackEntry {
            number,
            position,
            state: "open".into(),
            head_ref: head.into(),
            base_ref: base.into(),
            can_update: true,
        }
    }

    #[test]
    fn linear_open_stack_qualifies() {
        let es = [entry(1, 1, "a", "main"), entry(2, 2, "b", "a")];
        let (trunk, branches) = shape(&es, Some("main")).unwrap();
        assert_eq!(trunk, "main");
        assert_eq!(branches, vec!["a", "b"]);
    }

    #[test]
    fn merged_parent_gap_and_broken_chain_are_refused() {
        let mut es = [entry(1, 1, "a", "main"), entry(2, 2, "b", "a")];
        es[0].state = "merged".into();
        assert!(shape(&es, Some("main")).unwrap_err()[0].contains("merged"));

        let es = [entry(1, 1, "a", "main"), entry(3, 3, "c", "b")];
        assert!(shape(&es, Some("main")).is_err());

        let es = [entry(1, 1, "a", "dev")];
        assert!(shape(&es, Some("main")).unwrap_err()[0].contains("not main"));
    }
}
