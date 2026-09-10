//! Session event types for the lightweight session event store.

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
