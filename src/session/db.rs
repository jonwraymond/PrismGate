//! SQLite + FTS5 persistence for session events, owned by a background actor thread.
//!
//! `rusqlite::Connection` is `Send` but not `Sync`, and the gateway clones
//! `GateminiServer` across tasks, so we hand the connection to a dedicated
//! thread and proxy calls through an mpsc channel.

use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde::Serialize;

use crate::session::event::SessionEvent;

#[derive(Debug, Clone, Serialize)]
pub struct SessionSearchHit {
    pub session_key: String,
    pub category: String,
    pub event_type: String,
    pub name: String,
    pub payload: Option<String>,
    pub outcome: Option<String>,
    pub timestamp: String,
    pub score: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_avoided: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_returned: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResumeCard {
    pub session_key: String,
    pub event_count: usize,
    pub open_handles: Vec<String>,
    pub recent_tools: Vec<String>,
    pub decisions: Vec<String>,
    pub constraints: Vec<String>,
    pub notes: Vec<String>,
    pub lookup: &'static str,
}

enum Command {
    Record(SessionEvent),
    Search {
        query: String,
        limit: usize,
        reply: Sender<Result<Vec<SessionSearchHit>>>,
    },
    Resume {
        session_key: String,
        category: Option<String>,
        limit: usize,
        reply: Sender<Result<Vec<SessionSearchHit>>>,
    },
    ResumeCard {
        session_key: String,
        reply: Sender<Result<ResumeCard>>,
    },
    Purge {
        session_key: String,
    },
    Shutdown,
}

/// Cloneable handle to the session event store actor.
pub struct SessionEventStore {
    tx: Sender<Command>,
    _handle: std::sync::Arc<JoinHandle<()>>,
    refcount: std::sync::Arc<AtomicUsize>,
}

impl SessionEventStore {
    pub fn open(db_path: PathBuf) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("gatemini-session-store".to_string())
            .spawn(move || {
                if let Err(e) = run_actor(db_path, rx) {
                    tracing::error!(error = %e, "session event store actor failed");
                }
            })
            .context("spawn session store actor")?;
        Ok(Self {
            tx,
            _handle: std::sync::Arc::new(handle),
            refcount: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(1)),
        })
    }

    #[tracing::instrument(skip(self, event), fields(session = %event.session_key, category = %event.category, name = %event.name))]
    pub async fn record(&self, event: SessionEvent) -> Result<()> {
        self.tx
            .send(Command::Record(event.redact_secrets()))
            .context("session store channel closed")?;
        Ok(())
    }

    #[tracing::instrument(skip(self), fields(query_len = query.len(), limit))]
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<SessionSearchHit>> {
        let (reply, rx) = std::sync::mpsc::channel();
        self.tx
            .send(Command::Search {
                query: query.to_string(),
                limit,
                reply,
            })
            .context("session store channel closed")?;
        tokio::task::spawn_blocking(move || rx.recv())
            .await
            .context("session store join")??
    }

    pub async fn resume_summary(
        &self,
        session_key: &str,
        category: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SessionSearchHit>> {
        let (reply, rx) = std::sync::mpsc::channel();
        self.tx
            .send(Command::Resume {
                session_key: session_key.to_string(),
                category: category.map(str::to_owned),
                limit,
                reply,
            })
            .context("session store channel closed")?;
        tokio::task::spawn_blocking(move || rx.recv())
            .await
            .context("session store join")??
    }

    pub async fn resume_card(&self, session_key: &str) -> Result<ResumeCard> {
        let (reply, rx) = std::sync::mpsc::channel();
        self.tx
            .send(Command::ResumeCard {
                session_key: session_key.to_string(),
                reply,
            })
            .context("session store channel closed")?;
        tokio::task::spawn_blocking(move || rx.recv())
            .await
            .context("session store join")??
    }

    #[tracing::instrument(skip(self), fields(session = %session_key))]
    pub async fn purge_session(&self, session_key: &str) -> Result<()> {
        self.tx
            .send(Command::Purge {
                session_key: session_key.to_string(),
            })
            .context("session store channel closed")?;
        Ok(())
    }
}

impl Clone for SessionEventStore {
    fn clone(&self) -> Self {
        self.refcount
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self {
            tx: self.tx.clone(),
            _handle: self._handle.clone(),
            refcount: self.refcount.clone(),
        }
    }
}

impl Default for SessionEventStore {
    fn default() -> Self {
        Self::open(std::env::temp_dir().join("gatemini-session-events.sqlite3"))
            .expect("default session store open")
    }
}

impl Drop for SessionEventStore {
    fn drop(&mut self) {
        if self
            .refcount
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed)
            == 1
        {
            let _ = self.tx.send(Command::Shutdown);
        }
    }
}

fn run_actor(db_path: PathBuf, rx: Receiver<Command>) -> Result<()> {
    let conn = Connection::open(db_path)?;
    migrate(&conn)?;
    while let Ok(cmd) = rx.recv() {
        match cmd {
            Command::Record(event) => {
                if let Err(e) = record(&conn, &event) {
                    tracing::warn!(error = %e, "failed to record session event");
                }
            }
            Command::Search {
                query,
                limit,
                reply,
            } => {
                let result = search(&conn, &query, limit);
                let _ = reply.send(result);
            }
            Command::Resume {
                session_key,
                category,
                limit,
                reply,
            } => {
                let result = resume_summary(&conn, &session_key, category.as_deref(), limit);
                let _ = reply.send(result);
            }
            Command::ResumeCard { session_key, reply } => {
                let result = resume_card(&conn, &session_key);
                let _ = reply.send(result);
            }
            Command::Purge { session_key } => {
                if let Err(e) = conn.execute(
                    "DELETE FROM session_events WHERE session_key = ?1",
                    params![session_key],
                ) {
                    tracing::warn!(error = %e, "failed to purge session events");
                }
            }
            Command::Shutdown => break,
        }
    }
    Ok(())
}

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        CREATE TABLE IF NOT EXISTS session_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_key TEXT NOT NULL,
            category TEXT NOT NULL,
            event_type TEXT NOT NULL,
            name TEXT NOT NULL,
            payload TEXT,
            outcome TEXT,
            timestamp TEXT NOT NULL,
            handle TEXT,
            bytes_avoided INTEGER,
            bytes_returned INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_session_events_session_key ON session_events(session_key);
        CREATE INDEX IF NOT EXISTS idx_session_events_handle ON session_events(handle);
        CREATE VIRTUAL TABLE IF NOT EXISTS session_events_fts USING fts5(
            category,
            event_type,
            name,
            payload,
            outcome,
            content='session_events',
            content_rowid='id'
        );
        CREATE TRIGGER IF NOT EXISTS session_events_ai AFTER INSERT ON session_events BEGIN
            INSERT INTO session_events_fts(rowid, category, event_type, name, payload, outcome)
            VALUES (new.id, new.category, new.event_type, new.name, new.payload, new.outcome);
        END;
        CREATE TRIGGER IF NOT EXISTS session_events_ad AFTER DELETE ON session_events BEGIN
            INSERT INTO session_events_fts(session_events_fts, rowid, category, event_type, name, payload, outcome)
            VALUES ('delete', old.id, old.category, old.event_type, old.name, old.payload, old.outcome);
        END;
        CREATE TRIGGER IF NOT EXISTS session_events_au AFTER UPDATE ON session_events BEGIN
            INSERT INTO session_events_fts(session_events_fts, rowid, category, event_type, name, payload, outcome)
            VALUES ('delete', old.id, old.category, old.event_type, old.name, old.payload, old.outcome);
            INSERT INTO session_events_fts(rowid, category, event_type, name, payload, outcome)
            VALUES (new.id, new.category, new.event_type, new.name, new.payload, new.outcome);
        END;
        "#,
    )?;
    let _ = conn.execute("ALTER TABLE session_events ADD COLUMN handle TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE session_events ADD COLUMN bytes_avoided INTEGER",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE session_events ADD COLUMN bytes_returned INTEGER",
        [],
    );
    Ok(())
}

fn record(conn: &Connection, event: &SessionEvent) -> Result<()> {
    conn.execute(
        "INSERT INTO session_events(session_key, category, event_type, name, payload, outcome, timestamp, handle, bytes_avoided, bytes_returned) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            event.session_key,
            event.category,
            event.event_type,
            event.name,
            event.payload,
            event.outcome,
            event.timestamp,
            event.handle,
            event.bytes_avoided.map(|v| v as i64),
            event.bytes_returned.map(|v| v as i64),
        ],
    )?;
    Ok(())
}

fn search(conn: &Connection, query: &str, limit: usize) -> Result<Vec<SessionSearchHit>> {
    let limit = limit.clamp(1, 200);
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT s.session_key, s.category, s.event_type, s.name, s.payload, s.outcome, s.timestamp, f.rank, s.handle, s.bytes_avoided, s.bytes_returned
         FROM session_events_fts f
         JOIN session_events s ON s.id = f.rowid
         WHERE session_events_fts MATCH ?1
         ORDER BY f.rank
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![query, limit as i64], |row| {
        Ok(SessionSearchHit {
            session_key: row.get(0)?,
            category: row.get(1)?,
            event_type: row.get(2)?,
            name: row.get(3)?,
            payload: row.get(4)?,
            outcome: row.get(5)?,
            timestamp: row.get(6)?,
            score: row.get(7)?,
            handle: row.get(8)?,
            bytes_avoided: row.get::<_, Option<i64>>(9)?.map(|v| v as u64),
            bytes_returned: row.get::<_, Option<i64>>(10)?.map(|v| v as u64),
        })
    })?;
    let mut hits = Vec::new();
    for row in rows {
        hits.push(row?);
    }
    Ok(hits)
}

fn resume_summary(
    conn: &Connection,
    session_key: &str,
    category: Option<&str>,
    limit: usize,
) -> Result<Vec<SessionSearchHit>> {
    let limit = limit.clamp(1, 200);
    let mut stmt = if category.is_some() {
        conn.prepare_cached(
            "SELECT session_key, category, event_type, name, payload, outcome, timestamp, 0, handle, bytes_avoided, bytes_returned
             FROM session_events
             WHERE session_key = ?1 AND category = ?2
             ORDER BY id DESC
             LIMIT ?3",
        )?
    } else {
        conn.prepare_cached(
            "SELECT session_key, category, event_type, name, payload, outcome, timestamp, 0, handle, bytes_avoided, bytes_returned
             FROM session_events
             WHERE session_key = ?1
             ORDER BY id DESC
             LIMIT ?2",
        )?
    };
    let mut rows = if let Some(cat) = category {
        stmt.query(params![session_key, cat, limit as i64])?
    } else {
        stmt.query(params![session_key, limit as i64])?
    };
    let mut hits = Vec::new();
    while let Some(row) = rows.next()? {
        hits.push(SessionSearchHit {
            session_key: row.get(0)?,
            category: row.get(1)?,
            event_type: row.get(2)?,
            name: row.get(3)?,
            payload: row.get(4)?,
            outcome: row.get(5)?,
            timestamp: row.get(6)?,
            score: row.get(7)?,
            handle: row.get(8)?,
            bytes_avoided: row.get::<_, Option<i64>>(9)?.map(|v| v as u64),
            bytes_returned: row.get::<_, Option<i64>>(10)?.map(|v| v as u64),
        });
    }
    Ok(hits)
}

fn resume_card(conn: &Connection, session_key: &str) -> Result<ResumeCard> {
    let events = resume_summary(conn, session_key, None, 80)?;
    let mut open_handles = Vec::new();
    let mut recent_tools = Vec::new();
    let mut decisions = Vec::new();
    let mut constraints = Vec::new();
    let mut notes = Vec::new();
    for event in &events {
        if let Some(handle) = &event.handle
            && !open_handles.contains(handle)
        {
            open_handles.push(handle.clone());
        }
        match event.category.as_str() {
            "tool" if !recent_tools.contains(&event.name) => recent_tools.push(event.name.clone()),
            "decision" => {
                if let Some(payload) = &event.payload {
                    decisions.push(payload.clone());
                }
            }
            "constraint" => {
                if let Some(payload) = &event.payload {
                    constraints.push(payload.clone());
                }
            }
            "note" => {
                if let Some(payload) = &event.payload {
                    notes.push(payload.clone());
                }
            }
            _ => {}
        }
    }
    recent_tools.truncate(8);
    decisions.truncate(8);
    constraints.truncate(8);
    notes.truncate(8);
    open_handles.truncate(8);
    Ok(ResumeCard {
        session_key: session_key.to_string(),
        event_count: events.len(),
        open_handles,
        recent_tools,
        decisions,
        constraints,
        notes,
        lookup: "session_search(resume=true) then read_result(handle)",
    })
}
