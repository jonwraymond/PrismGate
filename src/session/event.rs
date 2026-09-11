//! Structured events emitted into the session event store.
//!
//! Keep this shape small and serializable. Large payloads belong behind a
//! result handle, not in the event row.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEvent {
    pub session_key: String,
    pub category: String,
    pub event_type: String,
    pub name: String,
    pub payload: Option<String>,
    pub outcome: Option<String>,
    pub timestamp: String,
    /// Optional retained-output handle. Survives compaction; fetch via read_result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    /// Optional byte accounting: raw bytes kept out of context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_avoided: Option<u64>,
    /// Optional byte accounting: bytes actually returned to the model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_returned: Option<u64>,
}

impl SessionEvent {
    pub fn with_handle(mut self, handle: impl Into<String>, total_bytes: u64) -> Self {
        self.handle = Some(handle.into());
        self.bytes_avoided = Some(total_bytes);
        self
    }

    pub fn with_bytes(mut self, returned: u64, processed: u64) -> Self {
        self.bytes_returned = Some(returned);
        self.bytes_avoided = Some(processed.saturating_sub(returned));
        self
    }
}
