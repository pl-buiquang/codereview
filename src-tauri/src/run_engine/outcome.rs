//! What a finished run amounts to: its outcome, final status and failure reason.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde_json::Value;

use crate::ai::{FinalResult, Harness};
use crate::ai_usage::RunUsage;

pub const STDERR_FILE: &str = "stderr.log";
pub const STDOUT_FILE: &str = "stdout.jsonl";
pub const PROMPT_FILE: &str = "system.txt";
const STDERR_MAX_BYTES: usize = 16_384;

#[derive(Default)]
pub struct Outcome {
    pub result: Option<Value>,
    pub usage: RunUsage,
    /// Why there is no usable result: spawn failure, preparation failure, exit status…
    pub failure: Option<String>,
    /// The host is quitting with this run left alive: nothing to settle, the next start adopts it.
    pub detached: bool,
}

impl Outcome {
    pub fn failed(msg: String) -> Self {
        Self { failure: Some(msg), ..Self::default() }
    }
}

/// Usage and failure reason of a run that produced `result` (if it did) and then ended with `exit`.
pub fn outcome_from(harness: &dyn Harness, result: Option<Value>, stderr: String, exit: Result<String, String>) -> Outcome {
    let usage = result.as_ref().map(|v| harness.final_result(v).usage).unwrap_or_default();
    let id = harness.id();
    let failure = match (&result, exit) {
        (Some(_), _) => None,
        (None, Ok(s)) => Some(format!("{id} exited with {s} without a result. {}", stderr.trim())),
        (None, Err(e)) => Some(format!("{id} process error: {e}")),
    };
    Outcome { result, usage, failure, detached: false }
}

pub fn read_stderr(dir: &Path) -> String {
    let mut buf = Vec::new();
    if let Ok(f) = File::open(dir.join(STDERR_FILE)) {
        let _ = f.take(STDERR_MAX_BYTES as u64).read_to_end(&mut buf);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

pub fn file_has_result(harness: &dyn Harness, path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|text| text.lines().rev().any(|l| harness.parse_line(l).0 == "result"))
        .unwrap_or(false)
}

/// A `NEEDS_ATTENTION:` line anywhere in the final message.
pub fn needs_attention(text: &str) -> bool {
    text.lines().any(|l| l.trim_start().starts_with("NEEDS_ATTENTION:"))
}

/// The status a finished run settles on. A stop request wins over whatever the process reported.
pub fn final_status(stop: Option<&'static str>, result: Option<&FinalResult>, text: &str) -> &'static str {
    if let Some(reason) = stop {
        return reason;
    }
    match result {
        None => "error",
        Some(r) if r.is_error => "error",
        Some(_) if needs_attention(text) => "needs_attention",
        Some(_) => "ok",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::ClaudeCode;
    use serde_json::json;

    #[test]
    fn needs_attention_is_a_line_prefix() {
        assert!(needs_attention("done\nNEEDS_ATTENTION: lint config is broken"));
        assert!(needs_attention("  NEEDS_ATTENTION: x"));
        assert!(!needs_attention("I will report NEEDS_ATTENTION: later"));
    }

    #[test]
    fn final_status_precedence() {
        let result = |v: Value| ClaudeCode.final_result(&v);
        let ok = result(json!({ "type": "result", "is_error": false, "result": "fine" }));
        let err = result(json!({ "type": "result", "is_error": true, "result": "boom" }));
        assert_eq!(final_status(None, Some(&ok), "fine"), "ok");
        assert_eq!(final_status(None, Some(&ok), "x\nNEEDS_ATTENTION: y"), "needs_attention");
        assert_eq!(final_status(None, Some(&err), "boom"), "error");
        assert_eq!(final_status(None, None, ""), "error");
        assert_eq!(final_status(Some("timeout"), Some(&ok), "fine"), "timeout");
        assert_eq!(final_status(Some("killed"), None, ""), "killed");
        // a stop beats even an error result
        assert_eq!(final_status(Some("killed"), Some(&err), "boom"), "killed");
    }

    #[test]
    fn file_has_result_looks_for_a_result_line() {
        let path = std::env::temp_dir().join(format!("cr_result_test_{}.jsonl", std::process::id()));
        std::fs::write(&path, "{\"type\":\"assistant\"}\n").unwrap();
        assert!(!file_has_result(&ClaudeCode, &path));
        std::fs::write(&path, "{\"type\":\"assistant\"}\n{\"type\":\"result\",\"result\":\"x\"}\n").unwrap();
        assert!(file_has_result(&ClaudeCode, &path));
        let _ = std::fs::remove_file(&path);
        assert!(!file_has_result(&ClaudeCode, &path));
    }

    #[test]
    fn outcome_without_a_result_reports_the_exit() {
        let o = outcome_from(&ClaudeCode, None, " boom \n".into(), Ok("exit status: 1".into()));
        assert_eq!(o.failure.as_deref(), Some("claude exited with exit status: 1 without a result. boom"));
        assert!(!o.detached);
        let ok = outcome_from(&ClaudeCode, Some(json!({"type": "result"})), String::new(), Ok("0".into()));
        assert!(ok.failure.is_none());
        let e = outcome_from(&ClaudeCode, None, String::new(), Err("gone".into()));
        assert_eq!(e.failure.as_deref(), Some("claude process error: gone"));
    }

    #[test]
    fn stream_fixture_kinds_and_final_status() {
        let h = &ClaudeCode;
        let lines = [
            (r#"{"type":"system","subtype":"init"}"#, "system"),
            (r#"{"type":"assistant","message":{}}"#, "assistant"),
            (r#"{"type":"user"}"#, "user"),
            (r#"{"x":1}"#, "unknown"),
            ("garbage", "raw"),
        ];
        for (line, kind) in lines {
            assert_eq!(h.parse_line(line).0, kind);
        }
        let ok = r#"{"type":"result","is_error":false,"result":"all good","num_turns":3,"session_id":"s","total_cost_usd":0.5,"usage":{"input_tokens":1,"output_tokens":2}}"#;
        let attention = r#"{"type":"result","is_error":false,"result":"x\nNEEDS_ATTENTION: look"}"#;
        let error = r#"{"type":"result","is_error":true,"result":"boom"}"#;
        let status = |line: &str| {
            let (kind, v) = h.parse_line(line);
            assert_eq!(kind, "result");
            let r = h.final_result(&v.unwrap());
            final_status(None, Some(&r), &r.text.clone().unwrap_or_default())
        };
        assert_eq!(status(ok), "ok");
        assert_eq!(status(attention), "needs_attention");
        assert_eq!(status(error), "error");
        let o = outcome_from(h, h.parse_line(ok).1, String::new(), Ok("0".into()));
        assert_eq!((o.usage.num_turns, o.usage.input_tokens, o.usage.output_tokens, o.usage.cost_usd), (3, 1, 2, 0.5));
    }
}
