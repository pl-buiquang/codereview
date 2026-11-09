//! Following a run's stdout file into numbered events.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::time::Duration;


use crate::ai::{norm_json, FinalResult, Harness, RunAcc};

pub const TAIL_POLL: Duration = Duration::from_millis(200);
/// Lines longer than this are stored as a stub (`truncated`, `bytes`) instead of verbatim.
pub const MAX_EVENT_BYTES: usize = 100_000;

pub enum Tail {
    /// The process ended and the file is drained: the last final event, if there was one.
    Done(Option<FinalResult>),
    /// The host is quitting with the run left alive.
    Detached,
}

/// The JSON stored for a line: the line itself, or a stub when it is too long.
pub fn event_json(kind: &str, line: &str) -> String {
    if line.len() > MAX_EVENT_BYTES {
        serde_json::json!({ "type": kind, "truncated": true, "bytes": line.len() }).to_string()
    } else {
        line.to_string()
    }
}

/// Follow `path` until `alive` turns false, calling `record(seq, kind, line)` for every line. The
/// first `skip` lines were recorded by an earlier session (the `seq` numbering is continuous across
/// adoption), so they are numbered but not recorded again. `detaching` is polled between reads.
pub fn tail_events(
    harness: &dyn Harness,
    path: &Path,
    skip: i64,
    alive: &mut dyn FnMut() -> bool,
    detaching: &dyn Fn() -> bool,
    record: &mut dyn FnMut(i64, &str, &str, &str),
) -> Tail {
    let Ok(file) = File::open(path) else { return Tail::Done(None) };
    let mut reader = BufReader::new(file);
    let mut result: Option<FinalResult> = None;
    let mut acc = RunAcc::default();
    let mut seq = 0i64;
    let mut pending = String::new();
    let mut handle = |line: &str, result: &mut Option<FinalResult>| {
        if line.trim().is_empty() {
            return;
        }
        let (kind, _) = harness.parse_line(line);
        // Normalized before the raw line is cut to a stub, and also for the lines recorded by an
        // earlier session: the final may be among them.
        let events = harness.normalize(line);
        seq += 1;
        if seq > skip {
            record(seq, &kind, line, &norm_json(&events, MAX_EVENT_BYTES));
        }
        acc.push(harness, line, &events);
        if let Some(fin) = events.iter().find_map(|e| e.as_final()) {
            *result = Some(acc.complete(fin));
        }
    };
    loop {
        // Sampled before reading, so output written just before the exit is still drained.
        let dead = !alive();
        loop {
            match reader.read_line(&mut pending) {
                Ok(0) | Err(_) => break,
                Ok(_) if pending.ends_with('\n') => {
                    handle(&pending, &mut result);
                    pending.clear();
                }
                Ok(_) => {}
            }
        }
        if dead {
            handle(&pending, &mut result);
            return Tail::Done(result);
        }
        if detaching() {
            return Tail::Detached;
        }
        std::thread::sleep(TAIL_POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::ClaudeCode;
    use serde_json::Value;

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("cr_tail_{name}_{}.jsonl", std::process::id()))
    }

    #[test]
    fn numbers_lines_skips_recorded_ones_and_returns_the_result() {
        let path = tmp("skip");
        std::fs::write(
            &path,
            "{\"type\":\"system\"}\n\n{\"type\":\"assistant\"}\ngarbage\n{\"type\":\"result\",\"result\":\"x\"}",
        )
        .unwrap();
        let mut seen = Vec::new();
        let tail = tail_events(&ClaudeCode, &path, 2, &mut || false, &|| false, &mut |s, k, _, _| seen.push((s, k.to_string())));
        // blank lines are not numbered; the first two lines were recorded earlier; an unterminated
        // last line is still drained once the process is gone
        assert_eq!(seen, vec![(3, "raw".to_string()), (4, "result".to_string())]);
        let Tail::Done(Some(r)) = tail else { panic!("expected a result") };
        assert_eq!(r.text.as_deref(), Some("x"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_file_is_done_without_result() {
        let tail = tail_events(&ClaudeCode, &tmp("none"), 0, &mut || false, &|| false, &mut |_, _, _, _| {});
        assert!(matches!(tail, Tail::Done(None)));
    }

    #[test]
    fn detaching_stops_the_tail_while_alive() {
        let path = tmp("detach");
        std::fs::write(&path, "{\"type\":\"assistant\"}\n").unwrap();
        let mut n = 0;
        let tail = tail_events(&ClaudeCode, &path, 0, &mut || true, &|| true, &mut |_, _, _, _| n += 1);
        assert!(matches!(tail, Tail::Detached));
        assert_eq!(n, 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn long_lines_are_stored_as_a_stub() {
        let long = "x".repeat(MAX_EVENT_BYTES + 1);
        let stub: Value = serde_json::from_str(&event_json("assistant", &long)).unwrap();
        assert_eq!(stub["truncated"], true);
        assert_eq!(stub["bytes"], MAX_EVENT_BYTES + 1);
        assert_eq!(event_json("assistant", "{}"), "{}");
    }
}
