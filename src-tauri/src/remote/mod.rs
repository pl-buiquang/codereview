//! The app side of remote agents (ADR 0007, S09): hosts, the SSH tunnel to their `cr worker`
//! daemon, a typed client of its `/v1` API, a [`harness::RemoteHarness`] and the mirror that copies
//! a remote run into the local store.
//!
//! The app uses the system `ssh` and stores no credential: whatever authenticates the connection
//! (agent, key, `~/.ssh/config`) is the user's.

pub mod host;
pub mod error;
pub mod tunnel;
pub mod http;
pub mod client;
#[cfg(all(test, feature = "worker", unix))]
mod test_server;
pub mod harness;
pub mod runtime;
pub mod startup;
pub mod mirror;
pub mod outbox;
pub mod launch;
pub mod sessions;
