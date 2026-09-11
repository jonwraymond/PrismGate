//! Retrieval helpers for the session event store.

use anyhow::Result;

use crate::session::db::{SessionEventStore, SessionSearchHit};

#[allow(dead_code)]
pub async fn search_events(
    store: &SessionEventStore,
    query: &str,
    limit: usize,
) -> Result<Vec<SessionSearchHit>> {
    store.search(query, limit).await
}

#[allow(dead_code)]
pub async fn resume_summary(
    store: &SessionEventStore,
    session_key: &str,
    category: Option<&str>,
    limit: usize,
) -> Result<Vec<SessionSearchHit>> {
    store.resume_summary(session_key, category, limit).await
}
