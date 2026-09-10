//! File-backed session event store with append-only JSONL, bounded retention,
//! and lightweight prefix search. No external native dependencies; built on
//! `serde_json` and existing runtime utilities only.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use dashmap::DashMap;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::session::event::SessionEvent;

const DEFAULT_MAX_EVENTS: usize = 1000;
const DEFAULT_TTL_SECONDS: u64 = 60 * 60;

#[derive(Debug, Clone, Serialize)]
pub struct SessionSearchHit {
    pub session_key: String,
    pub category: String,
    pub event_type: String,
    pub name: String,
    pub payload: Option<String>,
    pub outcome: Option<String>,
    pub timestamp: String,
    pub score: usize,
}

#[derive(Clone)]
pub struct SessionEventStore {
    stores: Arc<DashMap<PathBuf, Arc<RwLock<FileStore>>>>,
}

impl SessionEventStore {
    pub fn open(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(Self {
            stores: Arc::new(DashMap::new()),
        })
    }

    pub async fn record(&self, path: &PathBuf, event: &SessionEvent) -> Result<()> {
        let store = self.get_or_init(path.clone());
        let mut guard = store.write().await;
        guard.record(event)
    }

    pub async fn search(
        &self,
        path: &PathBuf,
        session_key: Option<&str>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SessionSearchHit>> {
        let store = self.get_or_init(path.clone());
        let guard = store.read().await;
        guard.search(session_key, query, limit)
    }

    pub async fn resume_summary(
        &self,
        path: &PathBuf,
        session_key: &str,
        category: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SessionSearchHit>> {
        let store = self.get_or_init(path.clone());
        let guard = store.read().await;
        guard.recent(session_key, category, limit)
    }

    pub async fn purge_session(&self, path: &PathBuf, session_key: &str) -> Result<()> {
        let store = self.get_or_init(path.clone());
        let mut guard = store.write().await;
        guard.purge_session(session_key)
    }

    fn get_or_init(&self, path: PathBuf) -> Arc<RwLock<FileStore>> {
        if let Some(entry) = self.stores.get(&path) {
            return Arc::clone(entry.value());
        }
        let store = Arc::new(RwLock::new(FileStore::new(path.clone())));
        self.stores.insert(path, Arc::clone(&store));
        store
    }
}

impl Default for SessionEventStore {
    fn default() -> Self {
        Self {
            stores: Arc::new(DashMap::new()),
        }
    }
}

struct FileStore {
    path: PathBuf,
    max_events: usize,
    ttl: std::time::Duration,
}

impl FileStore {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            max_events: DEFAULT_MAX_EVENTS,
            ttl: std::time::Duration::from_secs(DEFAULT_TTL_SECONDS),
        }
    }

    fn record(&mut self, event: &SessionEvent) -> Result<()> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(false)
            .write(true)
            .open(&self.path)
            .with_context(|| format!("open session store {}", self.path.display()))?;
        let mut writer = BufWriter::new(file);
        let line = serde_json::to_string(event)?;
        writer.write_all(line.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        self.enforce_limits()?;
        Ok(())
    }

    fn search(
        &self,
        session_key: Option<&str>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SessionSearchHit>> {
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(_) => return Ok(Vec::new()),
        };
        let reader = BufReader::new(file);
        let mut hits: Vec<SessionSearchHit> = Vec::new();
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
            let event: SessionEvent = match serde_json::from_str(&line) {
                Ok(e) => e,
                Err(_) => continue,
            };
            if let Some(sk) = session_key {
                if event.session_key != sk {
                    continue;
                }
            }
            if query.is_empty() {
                continue;
            }
            let text = format!(
                "{} {} {} {} {}",
                event.category,
                event.event_type,
                event.name,
                event.payload.clone().unwrap_or_default(),
                event.outcome.clone().unwrap_or_default()
            );
            let mut score: usize = 0;
            let q = query.to_lowercase();
            let lower = text.to_lowercase();
            if lower.contains(&q) {
                score += 1;
            }
            for token in q.split_whitespace() {
                if !token.is_empty() && lower.contains(token) {
                    score += 1;
                }
            }
            if score > 0 {
                hits.push(SessionSearchHit {
                    session_key: event.session_key,
                    category: event.category,
                    event_type: event.event_type,
                    name: event.name,
                    payload: event.payload,
                    outcome: event.outcome,
                    timestamp: event.timestamp,
                    score,
                });
            }
        }
        hits.sort_by(|a, b| b.score.cmp(&a.score));
        hits.truncate(limit);
        Ok(hits)
    }

    fn recent(
        &self,
        session_key: &str,
        category: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SessionSearchHit>> {
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(_) => return Ok(Vec::new()),
        };
        let reader = BufReader::new(file);
        let mut hits: Vec<SessionSearchHit> = Vec::new();
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
            let event: SessionEvent = match serde_json::from_str(&line) {
                Ok(e) => e,
                Err(_) => continue,
            };
            if event.session_key != session_key {
                continue;
            }
            if let Some(cat) = category {
                if event.category != cat {
                    continue;
                }
            }
            hits.push(SessionSearchHit {
                session_key: event.session_key,
                category: event.category,
                event_type: event.event_type,
                name: event.name,
                payload: event.payload,
                outcome: event.outcome,
                timestamp: event.timestamp,
                score: 0,
            });
        }
        hits.truncate(limit);
        Ok(hits)
    }

    fn purge_session(&mut self, session_key: &str) -> Result<()> {
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(_) => return Ok(()),
        };
        let reader = BufReader::new(file);
        let tmp_path = self.path.with_extension("tmp");
        {
            let tmp = File::create(&tmp_path)?;
            let mut writer = BufWriter::new(tmp);
            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => continue,
                };
                let event: SessionEvent = match serde_json::from_str(&line) {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                if event.session_key == session_key {
                    continue;
                }
                writer.write_all(line.as_bytes())?;
                writer.write_all(b"\n")?;
            }
            writer.flush()?;
        }
        fs::rename(&tmp_path, &self.path)?;
        Ok(())
    }

    fn enforce_limits(&mut self) -> Result<()> {
        if self.max_events == 0 {
            return Ok(());
        }
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(_) => return Ok(()),
        };
        let reader = BufReader::new(file);
        let mut lines: VecDeque<String> = VecDeque::new();
        for line in reader.lines() {
            if let Ok(l) = line {
                lines.push_back(l);
            }
        }
        if lines.len() > self.max_events {
            let excess = lines.len() - self.max_events;
            for _ in 0..excess {
                lines.pop_front();
            }
            let tmp_path = self.path.with_extension("tmp");
            let tmp = File::create(&tmp_path)?;
            let mut writer = BufWriter::new(tmp);
            for line in &lines {
                writer.write_all(line.as_bytes())?;
                writer.write_all(b"\n")?;
            }
            writer.flush()?;
            fs::rename(&tmp_path, &self.path)?;
        }
        Ok(())
    }
}
