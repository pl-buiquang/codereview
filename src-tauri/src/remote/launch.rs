//! The request a plain routine sends to a host: the prompt rendered locally, the neutral access and
//! tool policy (not Claude's native permission mode), the workspace named by repo and PR, and the
//! context snapshot.

use serde_json::Value;

use crate::ai::Access;
use crate::routines::{access_for_permission_mode, Routine};
use crate::worker::request::{AccessWire, RunRequest, ToolsWire, WorkspaceSpec};

pub struct LaunchInput<'a> {
    pub routine: &'a Routine,
    /// The harness id on the host.
    pub harness_id: &'a str,
    pub model: String,
    pub prompt: String,
    pub system_prompt: String,
    pub workspace: WorkspaceSpec,
    pub snapshot: Value,
    /// Stable per local run, so a retry after a crash finds the run it made.
    pub idempotency_key: String,
}

/// The neutral access of a routine's run for `harness_id`: a native Claude permission mode is mapped
/// back to the neutral level it stands for.
fn wire_access(access: &Access) -> AccessWire {
    match access {
        Access::Default => AccessWire::Default,
        Access::ReadOnly => AccessWire::ReadOnly,
        Access::Edit => AccessWire::Edit,
        Access::Full => AccessWire::Full,
        Access::Native(mode) => match access_for_permission_mode(mode) {
            "full" => AccessWire::Full,
            "edit" => AccessWire::Edit,
            _ => AccessWire::ReadOnly,
        },
    }
}

pub fn run_request(i: LaunchInput<'_>) -> RunRequest {
    let opts = i.routine.run_options(i.harness_id);
    let access = wire_access(&opts.access);
    let (tools, allowed_tools) = if opts.allowed_tools.is_empty() {
        let tools = match access {
            AccessWire::Full => ToolsWire::Full,
            AccessWire::Edit => ToolsWire::Edit,
            _ => ToolsWire::ReadOnly,
        };
        (tools, Vec::new())
    } else {
        (ToolsWire::Allowlist, opts.allowed_tools.clone())
    };
    RunRequest {
        idempotency_key: i.idempotency_key,
        harness: i.harness_id.to_string(),
        model: i.model,
        prompt: i.prompt,
        system_prompt: i.system_prompt,
        system_prompt_replace: opts.system_prompt_mode == crate::ai::SystemPromptMode::Replace,
        access,
        tools,
        allowed_tools,
        timeout_secs: i.routine.timeout_min.max(1) as u64 * 60,
        workspace: i.workspace,
        attachments: Vec::new(),
        context: i.snapshot,
    }
}

/// What a remote run drops from the routine's local configuration, for the log: the wire has no place
/// for harness-native flags.
pub fn dropped_settings(routine: &Routine, harness_id: &str) -> Vec<String> {
    let opts = routine.run_options(harness_id);
    if opts.extra_args.is_empty() { Vec::new() } else { vec![format!("extra_args {:?}", opts.extra_args)] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::request::WorkspaceKind;
    use serde_json::json;

    fn routine(overrides: Value, access: &str) -> Routine {
        Routine {
            id: 1,
            key: "k".into(),
            name: "K".into(),
            description: String::new(),
            context_type: "pr".into(),
            prompt_template: String::new(),
            system_prompt: String::new(),
            kind: "plain".into(),
            needs_worktree: true,
            access: access.into(),
            harness_overrides: overrides,
            profile_id: None,
            model: String::new(),
            timeout_min: 45,
            builtin: false,
            handler: None,
            enabled: true,
        }
    }

    fn input<'a>(r: &'a Routine, harness: &'a str) -> LaunchInput<'a> {
        LaunchInput {
            routine: r,
            harness_id: harness,
            model: "opus".into(),
            prompt: "do it".into(),
            system_prompt: "sys".into(),
            workspace: WorkspaceSpec { repo: "acme/widgets".into(), pr: Some(4), branch: Some("feat".into()), kind: WorkspaceKind::PrWorktree },
            snapshot: json!({"cards": []}),
            idempotency_key: "key-1".into(),
        }
    }

    #[test]
    fn a_native_claude_mode_becomes_the_neutral_level_and_the_allowlist_travels() {
        let r = routine(json!({"claude": {"permission_mode": "acceptEdits", "allowed_tools": ["Bash(git:*)", "Read"], "system_prompt_mode": "replace"}}), "read_only");
        let req = run_request(input(&r, "claude"));
        assert_eq!((req.access, req.tools), (AccessWire::Edit, ToolsWire::Allowlist));
        assert_eq!(req.allowed_tools, ["Bash(git:*)", "Read"]);
        assert!(req.system_prompt_replace);
        assert_eq!((req.timeout_secs, req.idempotency_key.as_str(), req.harness.as_str(), req.model.as_str()), (2700, "key-1", "claude", "opus"));
        assert_eq!(req.workspace.pr, Some(4));
        assert_eq!(req.context, json!({"cards": []}));
        let wire = serde_json::to_value(&req).unwrap();
        assert_eq!(wire["workspace"]["kind"], "pr-worktree");
        assert_eq!(wire["access"], "edit");
    }

    #[test]
    fn without_a_native_mode_or_allowlist_the_neutral_access_drives_the_tools() {
        for (access, tools, wire) in [("read_only", ToolsWire::ReadOnly, AccessWire::ReadOnly), ("edit", ToolsWire::Edit, AccessWire::Edit), ("full", ToolsWire::Full, AccessWire::Full)] {
            let r = routine(json!({}), access);
            let req = run_request(input(&r, "codex"));
            assert_eq!((req.access, req.tools), (wire, tools), "{access}");
            assert!(req.allowed_tools.is_empty() && !req.system_prompt_replace);
        }
        // a harness other than Claude never reads Claude's overrides
        let r = routine(json!({"claude": {"permission_mode": "bypassPermissions", "allowed_tools": ["Read"]}}), "read_only");
        let req = run_request(input(&r, "opencode"));
        assert_eq!((req.access, req.tools), (AccessWire::ReadOnly, ToolsWire::ReadOnly));
    }

    #[test]
    fn extra_args_are_reported_as_dropped() {
        let r = routine(json!({"claude": {"extra_args": ["--effort", "high"]}}), "edit");
        assert_eq!(dropped_settings(&r, "claude").len(), 1);
        assert!(dropped_settings(&r, "codex").is_empty());
    }
}
