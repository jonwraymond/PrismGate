# Session event store

PrismGate persists compact session events (tool calls, backend state, file reads)
in a SQLite database with FTS5 so they survive context compaction.

## How it works

- A dedicated actor thread owns the `rusqlite::Connection` (not Sync; cannot live on `GateminiServer` clones).
- `call_tool_chain` records ok/error events keyed by transport `session_id`.
- `session_search` queries FTS5 (BM25) or returns recent events with `resume=true`.
- `purge_session` also deletes this session's stored events.

## MCP tool

```
session_search
  query?: string       # FTS5 match; required unless resume=true
  resume?: bool        # most-recent events for this session
  category?: string    # tool | backend | file (resume mode)
  limit?: number       # default 20, max 200
```

## Default database

`/tmp/gatemini-session-events.sqlite3` (or the process temp dir).

The `session-store` cargo feature is **on by default**.
