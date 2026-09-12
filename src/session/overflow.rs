//! Overflow indexing: chunk large outputs into FTS5 so they stay searchable
//! without entering the conversation. Complements result handles.

use serde::Serialize;

use crate::session::db::SessionEventStore;
use crate::session::extract::{handle_event, timestamp_now};
use crate::session::redact::redact;
use crate::tools::retrieval::RetrievalOptions;

const TITLE_CHARS: usize = 80;

#[derive(Debug, Clone, Serialize)]
pub struct OverflowIndex {
    pub handle: String,
    pub sections: usize,
    pub total_bytes: usize,
    pub preview: Vec<String>,
    pub try_also: Vec<String>,
    pub lookup: &'static str,
}

/// Index `raw` into the session store as one handle event plus section rows.
/// Returns a compact pointer for the model. Never puts `raw` in the return value.
pub async fn index_overflow(
    store: &SessionEventStore,
    session_key: &str,
    handle: &str,
    raw: &str,
    source: &str,
    intent: Option<&str>,
) -> OverflowIndex {
    let total_bytes = raw.len() as u64;
    let _ = store
        .record(handle_event(session_key, handle, total_bytes, source))
        .await;

    let chunks = crate::tools::retrieval::chunk_text(raw);
    let mut titles: Vec<String> = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let title: String = chunk.chars().take(TITLE_CHARS).collect();
        let title = redact(&title.replace('\n', " "));
        titles.push(title.clone());
        let event = crate::session::event::SessionEvent {
            session_key: session_key.to_string(),
            category: "overflow".into(),
            event_type: "section".into(),
            name: format!("{handle}:{i}"),
            payload: Some(title),
            outcome: Some(source.to_string()),
            timestamp: timestamp_now(),
            handle: Some(handle.to_string()),
            bytes_avoided: Some(chunk.len() as u64),
            bytes_returned: Some(0),
        };
        let _ = store.record(event).await;
    }

    let preview = if let Some(intent) = intent.filter(|s| !s.trim().is_empty()) {
        let evidence = crate::tools::retrieval::retrieve(
            raw,
            &RetrievalOptions {
                queries: vec![intent.to_owned()],
                top_k: 5,
                neighbors: 0,
                max_bytes: 2_000,
            },
            2_000,
        );
        vec![redact(&evidence)]
    } else {
        titles.iter().take(5).cloned().collect()
    };

    let try_also = distinctive_terms(&redact(raw), 6);

    OverflowIndex {
        handle: handle.to_string(),
        sections: chunks.len().max(1),
        total_bytes: raw.len(),
        preview,
        try_also,
        lookup: "read_result(handle) or session_search(query)",
    }
}

fn distinctive_terms(text: &str, max: usize) -> Vec<String> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for token in crate::registry::tokenize(text) {
        if token.len() < 4 {
            continue;
        }
        *counts.entry(token).or_default() += 1;
    }
    let mut scored: Vec<(String, usize)> = counts.into_iter().collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    scored.into_iter().take(max).map(|(t, _)| t).collect()
}
