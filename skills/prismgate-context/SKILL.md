---
name: prismgate-context
description: >
  Keep large tool output out of the conversation. After compact or resume, call
  session_search(card=true) then read_result for listed handles. Use call_tool_chain
  to analyze data in the sandbox instead of pasting raw JSON/HTML/logs.
---

# PrismGate context discipline

PrismGate already retains large outputs behind handles and indexes them. Your job
is to **not** pull those bytes back into the prompt unless you need a specific page.

## After compact / new session / unclear state

Call this first, before asking the user what you were doing:

```
session_search(card=true)
```

Then:

- For each `open_handles` entry: `read_result(handle, limit=4096)` only if you need the payload.
- Honor `decisions` and `constraints` on the card. Do not re-litigate them.
- Persist new standing facts with `session_note(kind="decision"|"constraint"|"note", text=...)`.

Never dump a full retained payload into your reply.

## Analysis vs mutation

- **Analyze / count / filter / search / summarize** → `call_tool_chain` TypeScript. Return only the derived answer.
- **Need a slice of a retained result** → `read_result` with offset/limit or query.
- **Need to find a fact from earlier** → `session_search(query="...")`.
- **Mutate files / git write / install** → host tools as usual. Do not route those through the sandbox.

## Do not

- Re-run a mutating backend tool to recover output that already has a `result_handle`.
- Paste Playwright snapshots, `gh` issue dumps, or HTML into the conversation.
- Call `tool_info(detail="full")` until you are about to invoke that tool.
- Ignore the resume card after compact.

## Overflow pointers

When `call_tool_chain` returns `result_handle` + `sections` + `try_also`, the raw
bytes are indexed. Query with `session_search` or page with `read_result`. The
pointer is the context; the payload stays in the store.
