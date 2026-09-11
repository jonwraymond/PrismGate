//! Extraction helpers for session events.
//!
//! These are intentionally conservative and synchronous so they can
//! be called from hot paths without allocating large structures.

use crate::session::event::SessionEvent;

pub fn timestamp_now() -> String {
    let now = chrono::Utc::now();
    now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn tool_call_event(
    session_key: &str,
    tool_name: &str,
    backend: &str,
    outcome: &str,
) -> SessionEvent {
    SessionEvent {
        session_key: session_key.to_string(),
        category: "tool".into(),
        event_type: "tool_call".into(),
        name: tool_name.to_string(),
        payload: Some(format!("backend={backend}")),
        outcome: Some(outcome.to_string()),
        timestamp: timestamp_now(),
    }
}

#[allow(dead_code)]
pub fn backend_state_event(session_key: &str, backend: &str, state: &str) -> SessionEvent {
    SessionEvent {
        session_key: session_key.to_string(),
        category: "backend".into(),
        event_type: "backend_state".into(),
        name: backend.to_string(),
        payload: Some(state.to_string()),
        outcome: None,
        timestamp: timestamp_now(),
    }
}

#[allow(dead_code)]
pub fn file_rule_event(session_key: &str, file_path: &str) -> SessionEvent {
    SessionEvent {
        session_key: session_key.to_string(),
        category: "file".into(),
        event_type: "file_read".into(),
        name: file_path.to_string(),
        payload: None,
        outcome: None,
        timestamp: timestamp_now(),
    }
}
