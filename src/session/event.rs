//! Structured events emitted into the session event store.
//!
//! Keep this shape small and serializable. It is stored in SQLite
//! and exposed through FTS5 retrieval, so large payloads should be
//! summarized before recording.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SessionEvent {
    pub session_key: String,
    pub category: String,
    pub event_type: String,
    pub name: String,
    pub payload: Option<String>,
    pub outcome: Option<String>,
    pub timestamp: String,
}
