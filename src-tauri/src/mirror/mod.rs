//! The GitHub mirror (ADR 0002): one disposable local copy of GitHub PRs,
//! issues and users that Inbox, Board and Review all read. `fetch` talks to
//! GitHub, `store` to SQLite.

pub mod fetch;
pub mod store;

pub use fetch::{Node, FRAGMENTS};
