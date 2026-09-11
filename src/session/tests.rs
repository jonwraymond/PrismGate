//! Session event store tests (SQLite FTS5 actor).

use crate::session::db::SessionEventStore;
use crate::session::extract::tool_call_event;
use tempfile::NamedTempFile;

#[tokio::test]
async fn record_search_and_resume() {
    let tmp = NamedTempFile::new().unwrap();
    let store = SessionEventStore::open(tmp.path().to_path_buf()).unwrap();
    store
        .record(tool_call_event("s1", "search_tools", "gateway", "ok"))
        .await
        .unwrap();
    store
        .record(tool_call_event("s1", "call_tool_chain", "sandbox", "ok"))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let hits = store.search("search_tools", 10).await.unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().any(|h| h.name == "search_tools"));
    let recent = store.resume_summary("s1", Some("tool"), 10).await.unwrap();
    assert_eq!(recent.len(), 2);
}

#[tokio::test]
async fn purge_removes_session() {
    let tmp = NamedTempFile::new().unwrap();
    let store = SessionEventStore::open(tmp.path().to_path_buf()).unwrap();
    store
        .record(tool_call_event("s2", "search_tools", "gateway", "ok"))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    store.purge_session("s2").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let recent = store.resume_summary("s2", None, 10).await.unwrap();
    assert!(recent.is_empty());
}

#[tokio::test]
async fn resume_card_includes_handles_and_decisions() {
    let tmp = NamedTempFile::new().unwrap();
    let store = SessionEventStore::open(tmp.path().to_path_buf()).unwrap();
    store
        .record(crate::session::extract::handle_event(
            "s3",
            "r-abc",
            4096,
            "call_tool_chain",
        ))
        .await
        .unwrap();
    store
        .record(crate::session::extract::decision_event(
            "s3",
            "prefer handle lookup over rerun",
        ))
        .await
        .unwrap();
    store
        .record(crate::session::extract::constraint_event(
            "s3",
            "do not dump raw HTML",
        ))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let card = store.resume_card("s3").await.unwrap();
    assert_eq!(card.event_count, 3);
    assert!(card.open_handles.contains(&"r-abc".to_string()));
    assert!(card.decisions.iter().any(|d| d.contains("handle lookup")));
    assert!(card.constraints.iter().any(|c| c.contains("raw HTML")));
}

#[tokio::test]
async fn overflow_index_creates_searchable_sections() {
    let tmp = NamedTempFile::new().unwrap();
    let store = SessionEventStore::open(tmp.path().to_path_buf()).unwrap();
    let raw = "# Errors\n\nboom failed hard\n\n# Ok\n\nall good\n".repeat(20);
    let indexed = crate::session::overflow::index_overflow(
        &store.clone(),
        "s4",
        "r-overflow",
        &raw,
        "call_tool_chain",
        Some("errors"),
    )
    .await;
    assert!(indexed.sections >= 1);
    assert_eq!(indexed.handle, "r-overflow");
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    let hits = store.search("errors", 10).await.unwrap();
    assert!(
        hits.iter()
            .any(|h| h.handle.as_deref() == Some("r-overflow"))
    );
}
