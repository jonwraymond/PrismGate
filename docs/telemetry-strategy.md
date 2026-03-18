# Telemetry Strategy

This page separates two things:

- what Gatemini already tracks in-process
- what would still need to be added for end-to-end OTEL-style observability

## Current in-process observability

The current code already provides four useful layers.

### Structured logging

`tracing` is used throughout the runtime for lifecycle, failure, and state-change logs.

### Call tracking

`src/tracker.rs` (`CallTracker`) already records:

- recent tool calls (ring buffer)
- per-tool usage counts
- per-backend latency histograms (HDR histogram, p50/p95/p99, 1 µs–10 min range)
- per-tool bytes returned after output reduction
- total bytes processed (raw, before reduction) across the session
- session start time for uptime tracking

The `record_bytes(tool_name, returned, processed)` method is called after every `call_tool_chain` output pass. `session_stats()` aggregates all of this into a `SessionStats` struct exposed via the `gatemini://stats` resource.

### Output reduction accounting

The output pipeline (smart truncation, JSON auto-chunking, uniform array collapse, intent filtering) feeds directly into byte tracking. This means `gatemini://stats` shows real-time context savings for the current session without any external tooling.

### Backend health state

`src/backend/health.rs` already keeps:

- consecutive failure counters
- restart counters
- restart windows
- circuit-open timestamps

## What is not implemented yet

The repo does not currently expose:

- OTLP exporters
- OpenTelemetry spans
- payload-size histograms for discovery responses (`search_tools`, `tool_info`, `list_tools_meta`)
- explicit token counts
- session-level discovery-depth analytics

## Practical next step

The output pipeline already feeds byte data into `CallTracker`. The gap is on the discovery side: `search_tools`, `tool_info`, and `list_tools_meta` do not yet call `record_bytes`. Adding that would let you compare brief and full responses using the actual registry in production rather than documentation examples, and it would surface discovery overhead alongside execution overhead in `gatemini://stats`.

## Proposed OTEL shape

If OTEL is added later, the existing architecture suggests this hierarchy:

```text
session
  mcp.request
    discovery
    registration
    tool_call
      parse
      backend.call
  health_check
```

Useful fields would include:

- tool name
- backend name
- discovery mode
- result count
- response size (bytes_processed, bytes_returned)
- reduction percentage
- execution mode
- backend latency

## Why this page stays conservative

Older versions of the docs described a telemetry system as if it already existed. It does not. The tracker, byte accounting, and health state are real; OTEL export remains future work.
