//! Wire types of the API (`/v1`), serde only.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ai::{Access, ToolPolicy};

/// Neutral access level on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessWire {
    Default,
    #[default]
    ReadOnly,
    Edit,
    Full,
}

impl AccessWire {
    pub fn access(self) -> Access {
        match self {
            AccessWire::Default => Access::Default,
            AccessWire::ReadOnly => Access::ReadOnly,
            AccessWire::Edit => Access::Edit,
            AccessWire::Full => Access::Full,
        }
    }
}

/// Neutral tool policy on the wire; `allowlist` uses `allowed_tools`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolsWire {
    None,
    #[default]
    ReadOnly,
    Edit,
    Full,
    Allowlist,
}

impl ToolsWire {
    pub fn policy(self, allowed: &[String]) -> ToolPolicy {
        match self {
            ToolsWire::None => ToolPolicy::None,
            ToolsWire::ReadOnly => ToolPolicy::ReadOnly,
            ToolsWire::Edit => ToolPolicy::Edit,
            ToolsWire::Full => ToolPolicy::Full,
            ToolsWire::Allowlist => ToolPolicy::Allowlist(allowed.to_vec()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceKind {
    MainClone,
    PrWorktree,
    BranchWorktree,
}

/// Where a run happens, named by repo and PR / branch, never by a local path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSpec {
    /// `owner/name`
    pub repo: String,
    #[serde(default)]
    pub pr: Option<i64>,
    #[serde(default)]
    pub branch: Option<String>,
    pub kind: WorkspaceKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub content_base64: String,
}

fn default_timeout() -> u64 {
    30 * 60
}

/// `POST /v1/runs`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRequest {
    /// Client-supplied; a second request with the same key returns the first run.
    pub idempotency_key: String,
    pub harness: String,
    #[serde(default)]
    pub model: String,
    pub prompt: String,
    #[serde(default)]
    pub system_prompt: String,
    /// The system prompt replaces the harness' own instead of being appended.
    #[serde(default)]
    pub system_prompt_replace: bool,
    #[serde(default)]
    pub access: AccessWire,
    #[serde(default)]
    pub tools: ToolsWire,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    pub workspace: WorkspaceSpec,
    /// Written to a per-run scratch dir that is handed to the harness as an extra directory.
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// Snapshot answering `cr` reads inside the run (see `serve::remote`).
    #[serde(default)]
    pub context: Value,
}

/// `POST /v1/oneshot`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OneShotRequest {
    pub harness: String,
    #[serde(default)]
    pub model: String,
    pub prompt: String,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub system_prompt_replace: bool,
    #[serde(default)]
    pub access: AccessWire,
    #[serde(default)]
    pub tools: ToolsWire,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub max_turns: Option<u32>,
    /// Without a workspace the call runs in a scratch directory.
    #[serde(default)]
    pub workspace: Option<WorkspaceSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OneShotUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost_usd: f64,
}

/// Response of `/v1/oneshot`: `error` is set (and `text` empty) when the harness failed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OneShotResponse {
    pub text: String,
    pub usage: Option<OneShotUsage>,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_run_request_takes_safe_defaults() {
        let req: RunRequest = serde_json::from_str(
            r#"{"idempotency_key":"k","harness":"claude","prompt":"hi",
                "workspace":{"repo":"acme/widgets","kind":"main-clone"}}"#,
        )
        .unwrap();
        assert_eq!(req.access, AccessWire::ReadOnly);
        assert_eq!(req.tools, ToolsWire::ReadOnly);
        assert_eq!(req.timeout_secs, 1800);
        assert_eq!(req.workspace.kind, WorkspaceKind::MainClone);
        assert_eq!(req.tools.policy(&[]), ToolPolicy::ReadOnly);
        assert_eq!(ToolsWire::Allowlist.policy(&["Bash".into()]), ToolPolicy::Allowlist(vec!["Bash".into()]));
    }

    #[test]
    fn unknown_workspace_kind_is_rejected() {
        let r: Result<WorkspaceSpec, _> = serde_json::from_str(r#"{"repo":"a/b","kind":"clone-it"}"#);
        assert!(r.is_err());
    }
}
