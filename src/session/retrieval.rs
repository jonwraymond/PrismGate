//! Retrieval helpers for the lightweight session event store.

use anyhow::Result;
use serde::Serialize;

use crate::session::event::SessionEvent;
use crate::session_store::SessionSearchHit;

#[derive(Debug, Clone, Serialize)]
pub struct SessionSearchOptions {
    pub session_key: Option<String>,
    pub query: Option<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResumeSummaryOptions {
    pub session_key: String,
    pub category: Option<String>,
    pub limit: usize,
}

pub async fn search_events(
    store: &crate::session_store::SessionEventStore,
    db_path: &std::path::PathBuf,
    options: SessionSearchOptions,
) -> Result<Vec<SessionSearchHit>> {
    store
        .search(
            db_path,
            options.session_key.as_deref(),
            options.query.as_deref().unwrap_or(""),
            options.limit,
        )
        .await
}

pub async fn resume_summary(
    store: &crate::session_store::SessionEventStore,
    db_path: &std::path::PathBuf,
    options: ResumeSummaryOptions,
) -> Result<Vec<SessionSearchHit>> {
    store
        .resume_summary(
            db_path,
            &options.session_key,
            options.category.as_deref(),
            options.limit,
        )
        .await
}
