//! SQLite + FTS5 persistence for session events, owned by a background actor thread.
//!
//! `rusqlite::Connection` is `Send` but not `Sync`, and the gateway clones
//! `GateminiServer` across tasks, so we hand the connection to a dedicated
//! thread and proxy calls through an mpsc channel.

use std::path::PathBuf;
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
    Purge {
        session_key: String,
    },
    Shutdown,
}

/// Cloneable handle to the session event store actor.
#[derive(Clone)]
pub struct SessionEventStore {
    tx: Sender<Command>,
    _handle: std::sync::Arc<JoinHandle<()>>,
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
        })
    }

    pub async fn record(&self, event: SessionEvent) -> Result<()> {
        self.tx
            .send(Command::Record(event))
            .context("session store channel closed")?;
        Ok(())
    }

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

    pub async fn purge_session(&self, session_key: &str) -> Result<()> {
        self.tx
            .send(Command::Purge {
                session_key: session_key.to_string(),
            })
            .context("session store channel closed")?;
        Ok(())
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
        let _ = self.tx.send(Command::Shutdown);
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
            timestamp TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_session_events_session_key ON session_events(session_key);
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
    Ok(())
}

fn record(conn: &Connection, event: &SessionEvent) -> Result<()> {
    conn.execute(
        "INSERT INTO session_events(session_key, category, event_type, name, payload, outcome, timestamp) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            event.session_key,
            event.category,
            event.event_type,
            event.name,
            event.payload,
            event.outcome,
            event.timestamp,
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
        "SELECT s.session_key, s.category, s.event_type, s.name, s.payload, s.outcome, s.timestamp, f.rank
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
            "SELECT session_key, category, event_type, name, payload, outcome, timestamp, 0
             FROM session_events
             WHERE session_key = ?1 AND category = ?2
             ORDER BY id DESC
             LIMIT ?3",
        )?
    } else {
        conn.prepare_cached(
            "SELECT session_key, category, event_type, name, payload, outcome, timestamp, 0
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
        });
    }
    Ok(hits)
}
