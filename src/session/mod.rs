//! Session event store module.
//!
//! This module is gated behind the optional `session-store` feature.
//! It persists structured events from tool calls, backend state,
//! and session lifecycle into a SQLite database with FTS5 indexes.

#![cfg(feature = "session-store")]

pub mod db;
pub mod event;
pub mod extract;
pub mod overflow;
pub mod redact;
pub mod retrieval;

#[cfg(test)]
mod tests;

pub use db::SessionEventStore;
