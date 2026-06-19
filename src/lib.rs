//! Gatemini library crate.
//!
//! Re-exports all modules so the `gen_manpages` binary can reference the CLI
//! definitions as `gatemini::cli::{Cli, Command}`.

pub mod admin;
pub mod backend;
pub mod cache;
#[cfg(test)]
pub mod chaos_tests;
pub mod cli;
pub mod config;
#[cfg(feature = "semantic")]
pub mod embeddings;
pub mod integration_inventory;
pub mod ipc;
pub mod load_testing;
pub mod mcp_compliance_tests;
pub mod prompts;
pub mod rbac;
pub mod registry;
pub mod resources;
pub mod sandbox;
pub mod secrets;
pub mod server;
pub mod testutil;
pub mod tools;
pub mod tracker;

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Notify;

// Re-exports for use in tests and ipc/daemon.rs.
pub use crate::config::Config;
pub use crate::registry::ToolRegistry;
pub use crate::tracker::CallTracker;

/// Everything produced by shared initialization, ready for either direct or daemon mode.
#[derive(Clone)]
pub struct InitializedGateway {
    pub registry: Arc<ToolRegistry>,
    pub backend_manager: Arc<crate::backend::BackendManager>,
    pub tracker: Arc<CallTracker>,
    pub cache_path: PathBuf,
    pub config: Arc<Config>,
    pub shutdown_notify: Arc<Notify>,
}
