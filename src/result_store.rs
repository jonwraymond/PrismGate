//! Session-local retained tool results. Handles survive calls, not process restarts.

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

/// FIFO eviction, absolute TTL, and bounded total bytes/entry count.
/// Owned by one MCP connection and shared only with its clones.
pub struct ResultStore {
    entries: Mutex<VecDeque<Entry>>,
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
            entries: Mutex::new(VecDeque::new()),
            max_bytes,
            max_entries,
            ttl,
        }
    }

    pub fn insert(&self, raw: String) -> Option<String> {
        if raw.len() > self.max_bytes || self.max_entries == 0 || self.ttl.is_zero() {
            return None;
        }
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|e| e.created.elapsed() < self.ttl);
        let mut bytes: usize = entries.iter().map(|e| e.raw.len()).sum();
        while entries.len() >= self.max_entries || bytes + raw.len() > self.max_bytes {
            bytes -= entries.pop_front()?.raw.len();
        }
        let handle = format!("r-{:032x}", rand::random::<u128>());
        entries.push_back(Entry {
            handle: handle.clone(),
            raw,
            created: Instant::now(),
        });
        Some(handle)
    }

    pub fn expire(&self) -> usize {
        let mut entries = self.entries.lock().unwrap();
        let before = entries.len();
        entries.retain(|e| e.created.elapsed() < self.ttl);
        before - entries.len()
    }

    /// Case-sensitive literal search; offset enables repeated searches without raw replay.
    pub fn search(
        &self,
        handle: &str,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Option<ResultPage> {
        self.expire();
        let entries = self.entries.lock().unwrap();
        let raw = &entries.iter().find(|e| e.handle == handle)?.raw;
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
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|e| e.created.elapsed() < self.ttl);
        let raw = &entries.iter().find(|e| e.handle == handle)?.raw;
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
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|e| e.created.elapsed() < self.ttl);
        entries
            .iter()
            .find(|e| e.handle == handle)
            .map(|e| e.raw.clone())
    }
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
            .entries
            .lock()
            .unwrap()
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
}
