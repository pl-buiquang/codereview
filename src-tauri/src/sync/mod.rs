//! The sync engine (ADR 0003): brings the GitHub mirror up to date. One
//! pipeline (`pipeline::run`) shared by the app and the `cr` CLI; the app adds
//! a scheduler (`scheduler`) that decides when to run it.

pub mod inflight;
pub mod pipeline;
pub mod scheduler;

use serde::{Deserialize, Serialize};

pub use pipeline::run;
pub use scheduler::SyncEngine;

/// What a sync covers. `All` is Inbox + Board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Inbox,
    Board,
    All,
}

impl Scope {
    pub fn inbox(self) -> bool {
        matches!(self, Scope::Inbox | Scope::All)
    }

    pub fn board(self) -> bool {
        matches!(self, Scope::Board | Scope::All)
    }

    pub fn covers(self, other: Scope) -> bool {
        self == Scope::All || self == other
    }
}

/// - `Light`: probe, then fetch only what changed. Discovery only if asked.
/// - `Forced` (a Sync click): discovery + probe, and always refetch core,
///   status and threads of the visible entities.
/// - `Full`: discovery, and refetch every facet of every entity, ignoring
///   change keys; also fetches every clone and drops in-memory caches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Light,
    Forced,
    Full,
}

#[derive(Debug, Clone)]
pub struct SyncRequest {
    pub scope: Scope,
    pub mode: Mode,
    /// Entities on screen; `Forced` always refetches these.
    pub visible_ids: Vec<String>,
    /// Run discovery in a `Light` sync (the others always do).
    pub discover: bool,
}

impl SyncRequest {
    pub fn new(scope: Scope, mode: Mode) -> Self {
        Self { scope, mode, visible_ids: Vec::new(), discover: false }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub discovered: bool,
    pub viewer_login: String,
    /// Inbox items / board cards in the synced scope after the sync.
    pub item_count: i64,
    pub card_count: i64,
    pub closed_count: i64,
    pub auto_tracked: i64,
    pub probed: usize,
    /// Entities whose core + status facets were refetched.
    pub changed: usize,
    pub threads: usize,
    pub files: usize,
    pub duration_ms: i64,
}

/// Payload of the `mirror-updated` event.
#[derive(Debug, Clone, Serialize)]
pub struct MirrorUpdated {
    pub scopes: Vec<Scope>,
    pub ids: Vec<String>,
}

pub const MIRROR_UPDATED_EVENT: &str = "mirror-updated";
pub const SYNC_STATUS_EVENT: &str = "sync-status";
