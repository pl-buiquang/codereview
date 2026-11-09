//! `cr` inside a remote run (ADR 0007): writes land in the run's outbox, reads come from the context
//! snapshot, anything else says it is not available. Runs the real binary.

use std::path::Path;
use std::process::{Command, Output};

use codereview_lib::serve::remote::{write_snapshot, ENV_DB, ENV_DIR, ENV_RUN};
use codereview_lib::serve::store::ServeDb;
use serde_json::{json, Value};

struct Setup {
    dir: tempfile::TempDir,
    db: std::sync::Arc<ServeDb>,
    run: i64,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let db = ServeDb::open(&dir.path().join("serve.db")).unwrap();
    let (run, _) = db.insert_run("k", "claude", "", "{}", dir.path(), 60).unwrap();
    let ctx = json!({
        "cards": [{"card": {"repo": "acme/widgets", "number": 3, "title": "Fix it"}, "lane": "in_progress",
                   "meta": [{"id": 1, "kind": "note", "value": "remember"}]}],
        "focus": {"12": {"focus": {"status": "ready"}}}
    });
    write_snapshot(&dir.path().join("run"), &ctx).unwrap();
    Setup { dir, db, run }
}

fn cr(s: &Setup, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cr"))
        .args(args)
        // a database path that must never be created: remote mode does not open one
        .env("CODEREVIEW_DB", s.dir.path().join("must-not-exist.db"))
        .env(ENV_RUN, s.run.to_string())
        .env(ENV_DB, s.dir.path().join("serve.db"))
        .env(ENV_DIR, s.dir.path().join("run"))
        .current_dir(s.dir.path())
        .output()
        .unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

#[test]
fn board_and_focus_writes_and_notify_are_recorded_as_ops() {
    let s = setup();
    for args in [
        vec!["board", "meta", "add", "acme/widgets#3", "--kind", "note", "--value", "hi", "--tag", "plan"],
        vec!["board", "link", "card-1", "acme/widgets#9"],
        vec!["board", "meta", "rm", "4"],
        vec!["notify", "all done", "--title", "Run"],
    ] {
        let o = cr(&s, &args);
        assert!(o.status.success(), "{args:?}: {}", stderr(&o));
    }
    let plan = s.dir.path().join("plan.json");
    std::fs::write(&plan, r#"{"summary":"s","hunks":[]}"#).unwrap();
    let o = cr(&s, &["--json", "focus", "set", "12", "--file", plan.to_str().unwrap(), "--level", "focus"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(serde_json::from_str::<Value>(&stdout(&o)).unwrap()["opId"], format!("{}:5", s.run));

    let ops = s.db.outbox_after(s.run, 0).unwrap();
    let types: Vec<&str> = ops.iter().map(|o| o.op_type.as_str()).collect();
    assert_eq!(types, vec!["board_meta_add", "board_link", "board_meta_rm", "notify", "focus_set"]);
    assert_eq!(ops[0].args["reference"], "acme/widgets#3");
    assert_eq!(ops[0].args["tag"], "plan");
    assert_eq!(ops[1].args["issue_id"], "card-1");
    assert_eq!(ops[3].args["message"], "all done");
    assert_eq!(ops[4].args["plan"]["summary"], "s");
    // op ids are unique and stable: the same list comes back on a second read
    let ids: Vec<String> = ops.iter().map(|o| o.op_id.clone()).collect();
    assert_eq!(ids.iter().collect::<std::collections::BTreeSet<_>>().len(), 5);
    assert_eq!(s.db.outbox_after(s.run, 0).unwrap().iter().map(|o| o.op_id.clone()).collect::<Vec<_>>(), ids);
    assert!(!Path::new(&s.dir.path().join("must-not-exist.db")).exists());
}

#[test]
fn reads_are_answered_from_the_snapshot() {
    let s = setup();
    let o = cr(&s, &["board", "show", "acme/widgets#3"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(serde_json::from_str::<Value>(&stdout(&o)).unwrap()["card"]["title"], "Fix it");
    let o = cr(&s, &["board", "meta", "list", "acme/widgets#3"]);
    assert_eq!(serde_json::from_str::<Value>(&stdout(&o)).unwrap()[0]["value"], "remember");
    let o = cr(&s, &["board", "list", "--lane", "in_progress"]);
    assert_eq!(serde_json::from_str::<Value>(&stdout(&o)).unwrap().as_array().unwrap().len(), 1);
    let o = cr(&s, &["board", "list", "--lane", "done"]);
    assert!(serde_json::from_str::<Value>(&stdout(&o)).unwrap().as_array().unwrap().is_empty());
    let o = cr(&s, &["focus", "show", "12"]);
    assert_eq!(serde_json::from_str::<Value>(&stdout(&o)).unwrap()["focus"]["status"], "ready");
    let o = cr(&s, &["board", "show", "acme/widgets#99"]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("not in this remote run's context snapshot"), "{}", stderr(&o));
    assert!(s.db.outbox_after(s.run, 0).unwrap().is_empty(), "reads record nothing");
}

#[test]
fn other_commands_fail_with_a_clear_message() {
    let s = setup();
    for (args, name) in [(vec!["list"], "list"), (vec!["sync"], "sync"), (vec!["board", "sync"], "board sync"), (vec!["focus", "hunks", "1"], "focus hunks")] {
        let o = cr(&s, &args);
        assert!(!o.status.success(), "{args:?}");
        assert!(stderr(&o).contains(&format!("`cr {name}` is not available in a remote run")), "{args:?}: {}", stderr(&o));
    }
}

#[test]
fn notify_is_refused_outside_a_remote_run() {
    let dir = tempfile::tempdir().unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_cr"))
        .args(["notify", "x"])
        .env("CODEREVIEW_DB", dir.path().join("a.db"))
        .env_remove(ENV_RUN)
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("only available in a remote run"));
}
