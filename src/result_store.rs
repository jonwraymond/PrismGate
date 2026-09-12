//! Session-local retained tool results.
//!
//! Handles survive calls, but **not** process restarts.
//! Automatic call-result memoization is disabled: identical tool calls
//! always reach the backend so mutable or time-varying outputs stay fresh.
//! Large outputs can still be retained explicitly via `insert`, then read
//! back with `read`, `search`, or `raw`.

use serde::Serialize;

use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, Instant},
};

struct Entry {
    handle: String,
    raw: String,
    created: Instant,
}

struct Inner {
    entries: VecDeque<Entry>,
}

/// FIFO eviction, absolute TTL, and bounded total bytes/entry count.
/// Owned by one MCP connection and shared only with its clones.
pub struct ResultStore {
    inner: Mutex<Inner>,
    max_bytes: usize,
    max_entries: usize,
    ttl: Duration,
}

#[derive(Debug, Serialize)]
pub struct ResultPage {
    pub text: String,
    pub offset: usize,
    pub next_offset: Option<usize>,
    pub total_bytes: usize,
}

impl Default for ResultStore {
    fn default() -> Self {
        Self::new(32 * 1024 * 1024, 128, Duration::from_secs(1800))
    }
}

impl ResultStore {
    pub fn new(max_bytes: usize, max_entries: usize, ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(Inner {
                entries: VecDeque::new(),
            }),
            max_bytes,
            max_entries,
            ttl,
        }
    }

    /// Retain a raw payload under an explicit handle.
    pub fn insert(&self, raw: String) -> Option<String> {
        if raw.len() > self.max_bytes || self.max_entries == 0 || self.ttl.is_zero() {
            return None;
        }
        let mut inner = self.inner.lock().unwrap();
        prune_expired(&mut inner, self.ttl);
        let mut bytes: usize = inner.entries.iter().map(|e| e.raw.len()).sum();
        while inner.entries.len() >= self.max_entries || bytes + raw.len() > self.max_bytes {
            let dropped = inner.entries.pop_front()?;
            bytes -= dropped.raw.len();
        }
        let handle = format!("r-{:032x}", rand::random::<u128>());
        inner.entries.push_back(Entry {
            handle: handle.clone(),
            raw,
            created: Instant::now(),
        });
        Some(handle)
    }

    pub fn expire(&self) -> usize {
        let mut inner = self.inner.lock().unwrap();
        let before = inner.entries.len();
        prune_expired(&mut inner, self.ttl);
        before - inner.entries.len()
    }

    /// Case-sensitive literal search; offset enables repeated searches without raw replay.
    pub fn search(
        &self,
        handle: &str,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Option<ResultPage> {
        let mut inner = self.inner.lock().unwrap();
        prune_expired(&mut inner, self.ttl);
        let raw = &inner.entries.iter().find(|e| e.handle == handle)?.raw;
        if query.is_empty() || offset > raw.len() || !raw.is_char_boundary(offset) {
            return None;
        }
        let start = offset + raw[offset..].find(query)?;
        let end =
            raw.floor_char_boundary(start.saturating_add(limit.clamp(4, 16384)).min(raw.len()));
        Some(ResultPage {
            text: raw[start..end].into(),
            offset: start,
            next_offset: (end < raw.len()).then_some(end),
            total_bytes: raw.len(),
        })
    }

    pub fn read(&self, handle: &str, offset: usize, limit: usize) -> Option<ResultPage> {
        let mut inner = self.inner.lock().unwrap();
        prune_expired(&mut inner, self.ttl);
        let raw = &inner.entries.iter().find(|e| e.handle == handle)?.raw;
        if offset > raw.len() || !raw.is_char_boundary(offset) {
            return None;
        }
        let end =
            raw.floor_char_boundary(offset.saturating_add(limit.clamp(4, 16384)).min(raw.len()));
        Some(ResultPage {
            text: raw[offset..end].into(),
            offset,
            next_offset: (end < raw.len()).then_some(end),
            total_bytes: raw.len(),
        })
    }

    /// Full retained payload. Used for overflow indexing; not an MCP page.
    pub fn raw(&self, handle: &str) -> Option<String> {
        let mut inner = self.inner.lock().unwrap();
        prune_expired(&mut inner, self.ttl);
        inner
            .entries
            .iter()
            .find(|e| e.handle == handle)
            .map(|e| e.raw.clone())
    }

    /// Background TTL sweep so idle sessions still drop expired handles.
    ///
    /// Direct and daemon modes both spawn this; it never exits until the
    /// owning `Arc<ResultStore>` is dropped (the task holds a clone).
    pub fn spawn_janitor(store: std::sync::Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let expired = store.expire();
                if expired > 0 {
                    tracing::debug!(expired, "result store janitor purge");
                }
            }
        })
    }
}

fn prune_expired(inner: &mut Inner, ttl: Duration) {
    let live: Vec<String> = inner
        .entries
        .iter()
        .filter(|e| e.created.elapsed() < ttl)
        .map(|e| e.handle.clone())
        .collect();
    inner
        .entries
        .retain(|e| live.iter().any(|h| h == &e.handle));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expiration_limits_and_search() {
        let store = ResultStore::new(12, 2, Duration::from_secs(60));
        let first = store.insert("first".into()).unwrap();
        assert!(store.insert("x".repeat(13)).is_none());
        let second = store.insert("second".into()).unwrap();
        store.insert("third".into()).unwrap();
        assert!(store.read(&first, 0, 10).is_none());
        assert_eq!(store.search(&second, "cond", 0, 20).unwrap().text, "cond");
        assert!(store.search(&second, "absent", 0, 20).is_none());
        store
            .inner
            .lock()
            .unwrap()
            .entries
            .iter_mut()
            .for_each(|e| e.created = Instant::now() - Duration::from_secs(61));
        assert_eq!(store.expire(), 2);
        assert!(store.read(&second, 0, 10).is_none());
        let store = ResultStore::new(100, 1, Duration::from_secs(60));
        let h = store.insert("🦀hello".into()).unwrap();
        assert!(store.read(&h, 1, 10).is_none());
        assert_eq!(store.read(&h, 0, 1).unwrap().next_offset, Some(4));
        store.insert("next".into()).unwrap();
        assert!(store.read(&h, 0, 10).is_none());
    }

    #[test]
    fn insertion_and_cross_call_reuse() {
        let store = ResultStore::new(100, 2, std::time::Duration::from_secs(60));
        let handle = store.insert("hello world".into()).unwrap();
        assert_eq!(store.read(&handle, 0, 5).unwrap().text, "hello");
        assert_eq!(store.read(&handle, 6, 5).unwrap().text, "world");
        assert!(ResultStore::default().read(&handle, 0, 5).is_none());
    }

    #[test]
    fn janitor_expire_without_access() {
        let store = ResultStore::new(100, 4, Duration::from_secs(60));
        let a = store.insert("alpha".into()).unwrap();
        let b = store.insert("bravo".into()).unwrap();
        store
            .inner
            .lock()
            .unwrap()
            .entries
            .iter_mut()
            .for_each(|e| e.created = Instant::now() - Duration::from_secs(61));
        assert_eq!(store.expire(), 2);
        assert!(store.read(&a, 0, 10).is_none());
        assert!(store.read(&b, 0, 10).is_none());
        assert_eq!(store.expire(), 0);
    }

    #[test]
    fn repeated_identical_calls_always_retain_fresh_handles() {
        let store = ResultStore::new(1024, 8, Duration::from_secs(60));

        let first = store.insert("v1".into()).unwrap();
        let second = store.insert("v2".into()).unwrap();

        assert_eq!(store.raw(&first).as_deref(), Some("v1"));
        assert_eq!(store.raw(&second).as_deref(), Some("v2"));
        assert_eq!(store.read(&first, 0, 2).unwrap().text, "v1");
        assert_eq!(store.read(&second, 0, 2).unwrap().text, "v2");
    }
}
