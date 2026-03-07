# Telemetry Strategy

This page separates two things:

- what Gatemini already tracks in-process
- what would still need to be added for end-to-end OTEL-style observability

## Current in-process observability

The current code already provides three useful layers.

### Structured logging

`tracing` is used throughout the runtime for lifecycle, failure, and state-change logs.

### Call tracking

`src/tracker.rs` already records:

- recent tool calls
- per-tool usage counts
- per-backend latency histograms

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
- payload-size histograms for discovery responses
- explicit token counts
- session-level discovery-depth analytics

## Practical next step

If you want evidence for discovery efficiency without a full observability stack, the smallest useful addition would be response-size tracking around:

- `search_tools`
- `tool_info`
- `list_tools_meta`
- `read_resource` for `gatemini://tools`

That would let you compare brief and full responses using the actual registry in production rather than documentation examples.

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
- response size
- execution mode
- backend latency

## Why this page stays conservative

Older versions of the docs described a telemetry system as if it already existed. It does not. The tracker and health state are real; OTEL export remains future work.
