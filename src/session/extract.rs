//! Extraction helpers for session events.
//!
//! These are intentionally conservative and synchronous so they can
//! be called from hot paths without allocating large structures.

use crate::session::event::SessionEvent;
use crate::session::redact::redact;

pub fn timestamp_now() -> String {
    let now = chrono::Utc::now();
    now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn event(
    session_key: &str,
    category: &str,
    event_type: &str,
    name: &str,
    payload: Option<String>,
    outcome: Option<String>,
) -> SessionEvent {
    SessionEvent {
        session_key: session_key.to_string(),
        category: category.into(),
        event_type: event_type.into(),
        name: name.to_string(),
        payload: payload.map(|p| redact(&p)),
        outcome: outcome.map(|o| redact(&o)),
        timestamp: timestamp_now(),
        handle: None,
        bytes_avoided: None,
        bytes_returned: None,
    }
}

pub fn tool_call_event(
    session_key: &str,
    tool_name: &str,
    backend: &str,
    outcome: &str,
) -> SessionEvent {
    event(
        session_key,
        "tool",
        "tool_call",
        tool_name,
        Some(format!("backend={backend}")),
        Some(outcome.to_string()),
    )
}

#[allow(dead_code)]
pub fn backend_state_event(session_key: &str, backend: &str, state: &str) -> SessionEvent {
    event(
        session_key,
        "backend",
        "backend_state",
        backend,
        Some(state.to_string()),
        None,
    )
}

#[allow(dead_code)]
pub fn file_rule_event(session_key: &str, file_path: &str) -> SessionEvent {
    event(session_key, "file", "file_read", file_path, None, None)
}

pub fn decision_event(session_key: &str, summary: &str) -> SessionEvent {
    event(
        session_key,
        "decision",
        "decision",
        "decision",
        Some(summary.to_string()),
        None,
    )
}

pub fn constraint_event(session_key: &str, summary: &str) -> SessionEvent {
    event(
        session_key,
        "constraint",
        "constraint",
        "constraint",
        Some(summary.to_string()),
        None,
    )
}

pub fn note_event(session_key: &str, name: &str, summary: &str) -> SessionEvent {
    event(
        session_key,
        "note",
        "note",
        name,
        Some(summary.to_string()),
        None,
    )
}

pub fn handle_event(
    session_key: &str,
    handle: &str,
    total_bytes: u64,
    source: &str,
) -> SessionEvent {
    event(
        session_key,
        "handle",
        "result_handle",
        handle,
        Some(format!(
            "source={source} bytes={total_bytes} lookup=read_result"
        )),
        Some("retained".into()),
    )
    .with_handle(handle, total_bytes)
}
