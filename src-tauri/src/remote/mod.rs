//! The app side of remote agents (ADR 0007, S09): hosts, the SSH tunnel to their `cr worker`
//! daemon, a typed client of its `/v1` API, a [`harness::RemoteHarness`] and the mirror that copies
//! a remote run into the local store.
//!
//! The app uses the system `ssh` and stores no credential: whatever authenticates the connection
//! (agent, key, `~/.ssh/config`) is the user's.

pub mod client;
pub mod error;
pub mod harness;
pub mod host;
pub mod http;
pub mod launch;
pub mod mirror;
pub mod outbox;
pub mod runtime;
pub mod sessions;
pub mod startup;
#[cfg(all(test, feature = "worker", unix))]
mod test_server;
pub mod tunnel;
