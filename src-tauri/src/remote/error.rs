//! Errors of talking to a remote host. They reach the UI as `AppError::Other` strings of the form
//! `<code>: <message>`; the code is one of the API's (`unknown_repo`, `queue_full`, ...) or one of
//! the client's own ([`HOST_UNREACHABLE`], [`PROTOCOL_MISMATCH`], [`HOST_DISABLED`]).

use crate::error::AppError;

pub const HOST_UNREACHABLE: &str = "host_unreachable";
pub const PROTOCOL_MISMATCH: &str = "protocol_mismatch";
pub const HOST_DISABLED: &str = "host_disabled";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteError {
    pub code: String,
    pub message: String,
}

impl RemoteError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into() }
    }

    pub fn unreachable(message: impl Into<String>) -> Self {
        Self::new(HOST_UNREACHABLE, message)
    }

    pub fn disabled(message: impl Into<String>) -> Self {
        Self::new(HOST_DISABLED, message)
    }

    /// The remote spoke but said nothing sensible (bad JSON, unexpected status).
    pub fn protocol(message: impl Into<String>) -> Self {
        Self::new("bad_response", message)
    }

    /// The connection is the problem rather than the request: retrying later may work.
    pub fn is_connection(&self) -> bool {
        self.code == HOST_UNREACHABLE
    }
}

impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for RemoteError {}

impl From<RemoteError> for AppError {
    fn from(e: RemoteError) -> Self {
        AppError::Other(e.to_string())
    }
}

impl From<std::io::Error> for RemoteError {
    fn from(e: std::io::Error) -> Self {
        RemoteError::unreachable(e.to_string())
    }
}

pub type RemoteResult<T> = Result<T, RemoteError>;

/// A readable line for an error string (`host_unreachable: ...`) stored on a run or shown in the UI.
pub fn readable(error: &str) -> String {
    let (code, rest) = error.split_once(": ").unwrap_or(("", error));
    let hint = match code {
        HOST_UNREACHABLE => "The host could not be reached",
        PROTOCOL_MISMATCH => "The host runs an incompatible cr version",
        HOST_DISABLED => "The host is disabled",
        "unknown_repo" => "The repository is not registered on the host",
        "no_worktree" => "The host could not prepare a workspace",
        "harness_unavailable" => "The harness is not installed on the host",
        "capability_missing" => "The harness on the host cannot honour this run",
        "queue_full" => "The host is busy",
        "conflict" => "The host refused a conflicting request",
        "timeout" => "The host timed out",
        _ => return error.to_string(),
    };
    format!("{hint}: {rest}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_display_as_code_and_message_and_read_well() {
        let e = RemoteError::unreachable("ssh: connection refused");
        assert_eq!(e.to_string(), "host_unreachable: ssh: connection refused");
        assert!(e.is_connection() && !RemoteError::new("queue_full", "x").is_connection());
        assert_eq!(readable(&e.to_string()), "The host could not be reached: ssh: connection refused");
        assert_eq!(readable("unknown_repo: acme/x"), "The repository is not registered on the host: acme/x");
        assert_eq!(readable("something else"), "something else");
        assert_eq!(AppError::from(e).to_string(), "host_unreachable: ssh: connection refused");
    }
}
