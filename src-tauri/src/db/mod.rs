pub mod models;

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::error::AppResult;

/// Tauri-managed database handle. rusqlite is synchronous, so a Mutex around a
/// single connection is sufficient for a desktop single-user app.
pub struct Db(pub Mutex<Connection>);

/// Where the app's database lives, so child processes (`cr`) can share it.
pub struct DbPath(pub std::path::PathBuf);

/// Ordered list of migration SQL scripts. The current schema version is stored
/// in SQLite's `user_version` pragma; on open we apply any scripts beyond it.
const MIGRATIONS: &[&str] = &[
    include_str!("migrations/0001_init.sql"),
    include_str!("migrations/0002_file_view_state.sql"),
    include_str!("migrations/0003_comment_subject_type.sql"),
    include_str!("migrations/0004_comment_origin.sql"),
    include_str!("migrations/0005_repo_remote_index.sql"),
    include_str!("migrations/0006_inbox.sql"),
    include_str!("migrations/0007_comment_resolved.sql"),
    include_str!("migrations/0008_comment_anchored_base_sha.sql"),
    include_str!("migrations/0009_review_status_pending.sql"),
    include_str!("migrations/0010_review_sidebar_collapsed.sql"),
    include_str!("migrations/0011_chat.sql"),
    include_str!("migrations/0012_unified_repository.sql"),
    include_str!("migrations/0013_panel_widths.sql"),
    include_str!("migrations/0014_comment_github_thread_root_id.sql"),
    include_str!("migrations/0015_avatar_cache.sql"),
    include_str!("migrations/0016_focus.sql"),
    include_str!("migrations/0017_comment_tag.sql"),
    include_str!("migrations/0018_board.sql"),
    include_str!("migrations/0019_repo_friendly_name.sql"),
    include_str!("migrations/0020_board_ci_mergeable.sql"),
    include_str!("migrations/0021_board_merge_state.sql"),
    include_str!("migrations/0022_prompts_overview.sql"),
    include_str!("migrations/0023_board_auto_merge.sql"),
    include_str!("migrations/0024_worktree_gc_report.sql"),
    include_str!("migrations/0025_repo_sync.sql"),
    include_str!("migrations/0026_review_pin.sql"),
    include_str!("migrations/0027_gh_mirror.sql"),
];

pub fn open(path: &Path) -> AppResult<Connection> {
    let conn = Connection::open(path)?;
    // execute_batch tolerates the result row that `journal_mode` returns.
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
    migrate(&conn)?;
    Ok(conn)
}

pub(crate) fn migrate(conn: &Connection) -> AppResult<()> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let mut version = current as usize;
    while version < MIGRATIONS.len() {
        conn.execute_batch(MIGRATIONS[version])?;
        version += 1;
        // Per script, so a later failure can't leave applied scripts unrecorded.
        conn.pragma_update(None, "user_version", version as i64)?;
    }
    Ok(())
}

/// Open an in-memory, fully-migrated database for tests. Foreign keys are
/// enabled (so cascade-delete behaviour matches production); WAL is skipped as
/// it is meaningless for `:memory:`.
#[cfg(test)]
pub(crate) fn open_memory() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("enable foreign keys");
    migrate(&conn).expect("run migrations");
    conn
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_sets_user_version_and_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        let before: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(before, 0);

        migrate(&conn).unwrap();
        let after: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(after, MIGRATIONS.len() as i64);

        // Re-running migrate must be a no-op (no "table already exists" error).
        migrate(&conn).unwrap();
    }

    #[test]
    fn schema_creates_expected_tables() {
        let conn = open_memory();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap();
        let tables: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        for expected in ["comment", "repository", "review", "target", "board_card", "board_link", "board_meta"] {
            assert!(tables.contains(&expected.to_string()), "missing {expected}");
        }
    }

    #[test]
    fn migration_0007_upgrades_an_existing_0006_database() {
        // Apply every migration except the last (simulating a DB created at 0006),
        // insert a pre-0007 comment, then run the full migrate and confirm the new
        // column applies cleanly with the old row defaulting to NULL.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        for script in &MIGRATIONS[..MIGRATIONS.len() - 1] {
            conn.execute_batch(script).unwrap();
        }
        conn.pragma_update(None, "user_version", (MIGRATIONS.len() - 1) as i64)
            .unwrap();
        conn.execute(
            "INSERT INTO repository (local_path, default_branch, added_at) VALUES ('/r', 'main', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO target (repo_id, kind, title, base_ref, head_ref, created_at)
             VALUES (1, 'local', 't', 'a', 'b', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO review (target_id, body, status, created_at, updated_at)
             VALUES (1, '', 'draft', 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO comment (review_id, file_path, side, line, body, created_at, updated_at)
             VALUES (1, 'a.rs', 'RIGHT', 1, 'old', 'now', 'now')",
            [],
        )
        .unwrap();

        migrate(&conn).unwrap();

        let resolved: Option<String> = conn
            .query_row("SELECT resolved_at FROM comment WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert!(resolved.is_none(), "old comment defaults to unresolved");
    }

    #[test]
    fn migration_0008_upgrades_an_existing_0007_database() {
        // Apply every migration except the last (simulating a DB created at 0007),
        // insert a pre-0008 comment, then run the full migrate and confirm the new
        // anchored_base_sha column applies cleanly with the old row defaulting to NULL.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        for script in &MIGRATIONS[..MIGRATIONS.len() - 1] {
            conn.execute_batch(script).unwrap();
        }
        conn.pragma_update(None, "user_version", (MIGRATIONS.len() - 1) as i64)
            .unwrap();
        conn.execute(
            "INSERT INTO repository (local_path, default_branch, added_at) VALUES ('/r', 'main', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO target (repo_id, kind, title, base_ref, head_ref, created_at)
             VALUES (1, 'local', 't', 'a', 'b', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO review (target_id, body, status, created_at, updated_at)
             VALUES (1, '', 'draft', 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO comment (review_id, file_path, side, line, body, created_at, updated_at)
             VALUES (1, 'a.rs', 'LEFT', 1, 'old', 'now', 'now')",
            [],
        )
        .unwrap();

        migrate(&conn).unwrap();

        let base_sha: Option<String> = conn
            .query_row("SELECT anchored_base_sha FROM comment WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert!(base_sha.is_none(), "old comment defaults to NULL base pin");
    }

    #[test]
    fn migration_0026_backfills_review_pins_from_their_target() {
        // A DB at schema 25: two reviews share a target that holds the SHAs.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        for script in &MIGRATIONS[..25] {
            conn.execute_batch(script).unwrap();
        }
        conn.pragma_update(None, "user_version", 25_i64).unwrap();
        conn.execute(
            "INSERT INTO repository (local_path, default_branch, added_at) VALUES ('/r', 'main', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO target (repo_id, kind, title, base_ref, head_ref, base_sha, head_sha, created_at)
             VALUES (1, 'local', 't', 'a', 'b', 'base1', 'head1', 'now'),
                    (1, 'local', 'u', 'a', 'c', NULL, NULL, 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO review (target_id, body, status, created_at, updated_at)
             VALUES (1, '', 'draft', 'now', 'now'), (1, '', 'published', 'now', 'now'),
                    (2, '', 'draft', 'now', 'now')",
            [],
        )
        .unwrap();

        migrate(&conn).unwrap();

        let pins: Vec<(Option<String>, Option<String>)> = conn
            .prepare("SELECT base_sha, head_sha FROM review ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let both = (Some("base1".to_string()), Some("head1".to_string()));
        assert_eq!(pins, vec![both.clone(), both, (None, None)]);
    }

    #[test]
    fn migration_0027_moves_facts_into_the_mirror_and_keeps_user_state() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        for script in &MIGRATIONS[..26] {
            conn.execute_batch(script).unwrap();
        }
        conn.pragma_update(None, "user_version", 26_i64).unwrap();
        conn.execute_batch(
            "INSERT INTO repository (local_path, remote_owner, remote_name, added_at)
                 VALUES ('/r', 'acme', 'widget', 'added');
             INSERT INTO target (repo_id, kind, github_pr_number, title, base_ref, head_ref, base_sha, head_sha, created_at)
                 VALUES (1, 'github_pr', 7, 'old title', 'main', 'feat', 'b', 'h', 'now');
             INSERT INTO review (target_id, body, status, base_sha, head_sha, created_at, updated_at)
                 VALUES (1, '', 'draft', 'b', 'h', 'now', 'now');
             INSERT INTO items (id, type, number, repo, title, url, author_login, author_avatar, state,
                                body, updated_at, files_changed, top_files_json, ci_state,
                                untracked_at, engaged_at, first_seen_at, last_refreshed)
                 VALUES ('PR_7', 'pr', 7, 'Acme/Widget', 'inbox title', 'u7', 'octo', 'https://a/o.png', 'open',
                         'snippet', '2024-01-01', 3, '[{\"path\":\"a.rs\",\"additions\":2,\"deletions\":1,\"changes\":3}]',
                         'failure', NULL, 'engaged', 'seen', 'r1'),
                        ('I_9', 'issue', 9, 'other/repo', 'foreign issue', 'u9', NULL, NULL, 'open',
                         NULL, '2024-01-01', NULL, NULL, NULL, 'archived', NULL, 'seen', 'r1');
             INSERT INTO item_reasons (item_id, reason, detail) VALUES ('PR_7', 'direct_review', '');
             INSERT INTO board_card (id, type, repo, number, url, title, author_login, state, head_ref,
                                     has_reviews, pending_threads, updated_at, tracked_at, ci_status,
                                     mergeable, auto_merge)
                 VALUES ('PR_7', 'pr', 'acme/widget', 7, 'u7', 'board title', 'octo', 'open', 'feat',
                         1, 2, '2024-02-01', 'tracked', 'SUCCESS', 'MERGEABLE', 'SQUASH');
             INSERT INTO board_meta (card_id, kind, value, created_at) VALUES ('PR_7', 'note', 'n', 'now');",
        )
        .unwrap();

        migrate(&conn).unwrap();

        let q = |sql: &str| -> String {
            conn.query_row(&format!("SELECT CAST(({sql}) AS TEXT)"), [], |r| r.get::<_, Option<String>>(0))
                .unwrap()
                .unwrap_or_default()
        };
        assert_eq!(q("SELECT title FROM gh_pr WHERE node_id = 'PR_7'"), "board title", "newer updated_at wins");
        assert_eq!(q("SELECT body_snippet FROM gh_pr WHERE node_id = 'PR_7'"), "snippet");
        assert_eq!(q("SELECT head_ref || ' ' || base_ref FROM gh_pr WHERE node_id = 'PR_7'"), "feat main");
        assert_eq!(q("SELECT ci_state || ' ' || auto_merge_method FROM gh_pr_status WHERE pr_id = 'PR_7'"), "SUCCESS SQUASH");
        assert_eq!(q("SELECT pending_count FROM gh_pr_threads WHERE pr_id = 'PR_7'"), "2");
        assert_eq!(q("SELECT filename FROM gh_pr_file WHERE pr_id = 'PR_7'"), "a.rs");
        assert_eq!(q("SELECT avatar_url FROM gh_user WHERE login = 'octo'"), "https://a/o.png");
        assert_eq!(q("SELECT engaged_at FROM inbox_entry WHERE id = 'PR_7'"), "engaged");
        assert_eq!(q("SELECT untracked_at FROM inbox_entry WHERE id = 'I_9'"), "archived");
        assert_eq!(q("SELECT reason FROM item_reasons WHERE item_id = 'PR_7'"), "direct_review");
        assert_eq!(q("SELECT tracked_at FROM board_card WHERE id = 'PR_7'"), "tracked");
        assert_eq!(q("SELECT value FROM board_meta WHERE card_id = 'PR_7'"), "n");
        assert_eq!(q("SELECT gh_pr_id FROM target WHERE id = 1"), "PR_7");
        assert_eq!(q("SELECT head_sha FROM review WHERE id = 1"), "h", "pins untouched");
        assert_eq!(q("SELECT registered_at FROM repository WHERE id = 1"), "added");
        assert_eq!(
            q("SELECT COALESCE(registered_at, 'auto') FROM repository WHERE remote_owner = 'other'"),
            "auto",
            "a repo only the inbox knew is auto-added"
        );
        assert_eq!(q("SELECT title FROM gh_issue WHERE node_id = 'I_9'"), "foreign issue");
        assert_eq!(q("SELECT COUNT(*) FROM sqlite_master WHERE name = 'items'"), "0");
        let violations: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r.get(0))
            .unwrap();
        assert_eq!(violations, 0);
        conn.execute("DELETE FROM review", []).unwrap();
        conn.execute("DELETE FROM board_card WHERE id = 'PR_7'", []).unwrap();
        assert_eq!(q("SELECT COUNT(*) FROM board_meta"), "0", "board_meta still cascades from the rebuilt card");
    }

    #[test]
    fn migration_0009_rebuilds_review_preserving_rows() {
        // Apply migrations through 0008 (a DB created at schema version 8), seed
        // a review + comment + file_view_state, then apply the 0009 table rebuild
        // and confirm the rebuild preserved every row and broke no FK.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        for script in &MIGRATIONS[..8] {
            conn.execute_batch(script).unwrap();
        }
        conn.pragma_update(None, "user_version", 8_i64).unwrap();

        conn.execute(
            "INSERT INTO repository (path, default_branch, added_at) VALUES ('/r', 'main', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO target (repo_id, kind, title, base_ref, head_ref, created_at)
             VALUES (1, 'local', 't', 'a', 'b', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO review (id, target_id, body, status, created_at, updated_at)
             VALUES (42, 1, 'b', 'published', 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO comment (review_id, file_path, side, line, body, created_at, updated_at)
             VALUES (42, 'a.rs', 'RIGHT', 1, 'c', 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO file_view_state (review_id, file_path, viewed, updated_at)
             VALUES (42, 'a.rs', 1, 'now')",
            [],
        )
        .unwrap();

        conn.execute_batch(MIGRATIONS[8]).unwrap();

        let (id, status): (i64, String) = conn
            .query_row("SELECT id, status FROM review", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(id, 42, "review row keeps its id through the rebuild");
        assert_eq!(status, "published", "review status survives the rebuild");

        let comments: i64 = conn
            .query_row("SELECT COUNT(*) FROM comment", [], |r| r.get(0))
            .unwrap();
        assert_eq!(comments, 1, "comment rows are NOT cascade-deleted");
        let views: i64 = conn
            .query_row("SELECT COUNT(*) FROM file_view_state", [], |r| r.get(0))
            .unwrap();
        assert_eq!(views, 1, "file_view_state rows are NOT cascade-deleted");

        let violations: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(violations, 0, "no dangling foreign keys after the rebuild");
    }

    #[test]
    fn migration_0009_check_accepts_pending_rejects_bogus() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO repository (local_path, default_branch, added_at) VALUES ('/r', 'main', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO target (repo_id, kind, title, base_ref, head_ref, created_at)
             VALUES (1, 'local', 't', 'a', 'b', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO review (target_id, body, status, created_at, updated_at)
             VALUES (1, '', 'published_pending', 'now', 'now')",
            [],
        )
        .expect("published_pending passes the rebuilt CHECK");
        let bogus = conn.execute(
            "INSERT INTO review (target_id, body, status, created_at, updated_at)
             VALUES (1, '', 'bogus', 'now', 'now')",
            [],
        );
        assert!(bogus.is_err(), "an unknown status fails the CHECK");
    }

    #[test]
    fn migration_0009_cascade_still_works() {
        let conn = open_memory();
        conn.execute(
            "INSERT INTO repository (local_path, default_branch, added_at) VALUES ('/r', 'main', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO target (repo_id, kind, title, base_ref, head_ref, created_at)
             VALUES (1, 'local', 't', 'a', 'b', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO review (id, target_id, body, status, created_at, updated_at)
             VALUES (7, 1, '', 'draft', 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO comment (review_id, file_path, side, line, body, created_at, updated_at)
             VALUES (7, 'a.rs', 'RIGHT', 1, 'c', 'now', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO file_view_state (review_id, file_path, viewed, updated_at)
             VALUES (7, 'a.rs', 1, 'now')",
            [],
        )
        .unwrap();

        conn.execute("DELETE FROM review WHERE id = 7", []).unwrap();

        let comments: i64 = conn
            .query_row("SELECT COUNT(*) FROM comment", [], |r| r.get(0))
            .unwrap();
        assert_eq!(comments, 0, "deleting a review cascades to its comments");
        let views: i64 = conn
            .query_row("SELECT COUNT(*) FROM file_view_state", [], |r| r.get(0))
            .unwrap();
        assert_eq!(views, 0, "deleting a review cascades to file_view_state");
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let conn = open_memory();
        // Inserting a target for a non-existent repo must fail the FK constraint.
        let err = conn.execute(
            "INSERT INTO target (repo_id, kind, title, base_ref, head_ref, created_at)
             VALUES (999, 'local', 't', 'a', 'b', 'now')",
            [],
        );
        assert!(err.is_err(), "expected foreign key violation");
    }
}
