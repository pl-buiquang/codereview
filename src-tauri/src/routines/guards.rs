use rusqlite::{params, Connection};

use super::context::{pr_key, Light};
use super::{babysit_active, Routine, BABYSIT_KEY};
use crate::error::{AppError, AppResult};

type Guard = fn(&Connection, &Routine, &Light) -> AppResult<Option<String>>;

const GUARDS: &[Guard] = &[one_babysit_per_stack];

pub fn blocked(conn: &Connection, routine: &Routine, light: &Light) -> AppResult<Option<String>> {
    for guard in GUARDS {
        if let Some(reason) = guard(conn, routine, light)? {
            return Ok(Some(reason));
        }
    }
    Ok(None)
}

pub fn check(conn: &Connection, routine: &Routine, light: &Light) -> AppResult<()> {
    match blocked(conn, routine, light)? {
        Some(reason) => Err(AppError::Other(reason)),
        None => Ok(()),
    }
}

fn parse_pr_key(key: &str) -> Option<(&str, &str, i64)> {
    let (repo, number) = key.strip_prefix("pr:")?.rsplit_once('#')?;
    let (owner, name) = repo.split_once('/')?;
    Some((owner, name, number.parse().ok()?))
}

fn stack_siblings(conn: &Connection, owner: &str, name: &str, number: i64) -> AppResult<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT o.number FROM gh_pr p
         JOIN repository r ON r.id = p.repo_id
         JOIN gh_pr_status s ON s.pr_id = p.node_id
         JOIN gh_pr o ON o.repo_id = p.repo_id AND o.number != p.number
         JOIN gh_pr_status os ON os.pr_id = o.node_id AND os.stack_number = s.stack_number
         WHERE r.remote_owner = ?1 COLLATE NOCASE AND r.remote_name = ?2 COLLATE NOCASE
           AND p.number = ?3 AND s.stack_number IS NOT NULL
         ORDER BY os.stack_position",
    )?;
    let rows = stmt
        .query_map(params![owner, name, number], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(rows)
}

fn one_babysit_per_stack(
    conn: &Connection,
    routine: &Routine,
    light: &Light,
) -> AppResult<Option<String>> {
    if routine.key != BABYSIT_KEY {
        return Ok(None);
    }
    let Some((owner, name, number)) = parse_pr_key(&light.key) else {
        return Ok(None);
    };
    for sibling in stack_siblings(conn, owner, name, number)? {
        if babysit_active(conn, &pr_key(owner, name, sibling)) {
            return Ok(Some(format!(
                "{} is already running on {owner}/{name}#{sibling}, in the same stack. Stop it first: two babysits would race to restack the same branches.",
                routine.name
            )));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;
    use crate::mirror;

    fn seed_pr(conn: &Connection, number: i64, stack: Option<i64>) {
        let repo_id = mirror::store::ensure_repo(conn, "Owner", "Repo", "now").unwrap();
        let id = format!("pr{number}");
        conn.execute(
            "INSERT INTO gh_pr (node_id, repo_id, number, title, url, gh_updated_at, fetched_at, state)
             VALUES (?1, ?2, ?3, 't', 'u', 'now', 'now', 'open')",
            params![id, repo_id, number],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO gh_pr_status (pr_id, stack_number, stack_position, fetched_at)
             VALUES (?1, ?2, ?3, 'now')",
            params![id, stack, number],
        )
        .unwrap();
    }

    fn routine(conn: &Connection, key: &str) -> Routine {
        super::super::list(conn)
            .unwrap()
            .into_iter()
            .find(|r| r.key == key)
            .unwrap()
    }

    fn run_on(conn: &Connection, key: &str, number: i64, status: &str) {
        conn.execute(
            "INSERT INTO routine_run (routine_id, context_type, context_key, context_json, status, queued_at)
             VALUES (?1, 'pr', ?2, '{}', ?3, 'now')",
            params![routine(conn, key).id, pr_key("owner", "repo", number), status],
        )
        .unwrap();
    }

    fn light(number: i64) -> Light {
        Light {
            key: pr_key("owner", "repo", number),
            label: format!("owner/repo#{number}"),
            kinds: vec!["pr"],
        }
    }

    fn blocked_on(conn: &Connection, key: &str, number: i64) -> Option<String> {
        blocked(conn, &routine(conn, key), &light(number)).unwrap()
    }

    #[test]
    fn a_second_babysit_on_the_same_stack_is_blocked() {
        let conn = open_memory();
        super::super::seed(&conn).unwrap();
        for n in [1, 2, 3] {
            seed_pr(&conn, n, Some(7));
        }
        run_on(&conn, BABYSIT_KEY, 2, "running");
        let reason = blocked_on(&conn, BABYSIT_KEY, 3).unwrap();
        assert!(reason.contains("owner/repo#2"), "{reason}");
        assert!(blocked_on(&conn, BABYSIT_KEY, 1).is_some());
        assert!(check(&conn, &routine(&conn, BABYSIT_KEY), &light(3)).is_err());
    }

    #[test]
    fn other_stacks_runs_and_routines_are_not_blocked() {
        let conn = open_memory();
        super::super::seed(&conn).unwrap();
        seed_pr(&conn, 1, Some(7));
        seed_pr(&conn, 2, Some(7));
        seed_pr(&conn, 3, Some(8));
        seed_pr(&conn, 4, None);
        seed_pr(&conn, 5, None);
        run_on(&conn, BABYSIT_KEY, 1, "ok");
        assert_eq!(blocked_on(&conn, BABYSIT_KEY, 2), None);
        run_on(&conn, BABYSIT_KEY, 3, "running");
        run_on(&conn, BABYSIT_KEY, 4, "queued");
        assert_eq!(blocked_on(&conn, BABYSIT_KEY, 2), None);
        assert_eq!(blocked_on(&conn, BABYSIT_KEY, 5), None);
        run_on(&conn, "issue_implement", 1, "running");
        assert_eq!(blocked_on(&conn, BABYSIT_KEY, 2), None);
        run_on(&conn, BABYSIT_KEY, 1, "running");
        assert_eq!(blocked_on(&conn, "issue_implement", 2), None);
        assert!(blocked_on(&conn, BABYSIT_KEY, 2).is_some());
    }
}
