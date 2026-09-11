# Session event store

PrismGate persists compact session events in SQLite FTS5 so they survive
context compaction. Raw payloads stay behind result handles; the event
row is a pointer, not a dump.

## After compaction

1. `session_search(card=true)` — compact card: open handles, recent tools, decisions, constraints.
2. `read_result(handle)` — fetch retained payloads page-by-page. Do not rerun mutating tools.
3. `session_search(query="...")` — BM25 over events if you need a specific fact.

## Persist facts, not blobs

```
session_note(kind="decision", text="prefer handle lookup over rerun")
session_note(kind="constraint", text="do not dump raw HTML into context")
session_note(kind="note", name="plan", text="next: wire host skill")
```

`call_tool_chain` automatically records ok/error. If the response is a
`result_handle`, that handle is indexed on the event so the resume card
can list it.

## Tools

| Tool | Role |
|---|---|
| `session_search` | query / resume=true / card=true |
| `session_note` | durable decision/constraint/note |
| `read_result` | page a retained payload |
| `purge_session` | wipe tracker + this session's events |

## Resource

`gatemini://resume` reminds the model to call `session_search(card=true)`.

The `session-store` cargo feature is **on by default**.
