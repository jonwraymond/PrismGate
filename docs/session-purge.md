# Explicit clean slate

Run `gatemini purge --yes` against an already-running daemon, or use
`gatemini purge --yes --socket /path/to/daemon.sock` for a custom socket.
The command never starts or restarts the daemon. Missing confirmation is an
argument error; connection failures return a nonzero exit status. The operation
has a 15-second timeout; a timeout means the outcome is unknown.

MCP clients (including direct mode) can call `purge_session` with
`{"confirm": true}`. False confirmation is rejected without clearing anything.

## Scope

**The CallTracker is shared by all clients of a daemon.** Purge clears its recent
calls, usage counts, backend latency histograms, returned/processed byte counters,
and resets its statistics start time. It attempts to save the refreshed cache,
preserving tool schemas and embeddings; existing cache-save failures are logged
rather than propagated, so persistent cleanup is best effort.

Backend processes, backend health state, registrations, configuration, secrets,
and discovery flood-protection budgets are preserved. This does not erase the
client's conversation, backend-owned memory, or dedicated backend instance state.
No separate session-memory or result store exists in this implementation; future
stores must explicitly integrate their own scoped purge behavior.

Tracker mutations are serialized against reset. Calls that complete afterwards
can repopulate the tracker as new activity; purge is not cancellation of in-flight
work and does not promise a permanently empty tracker while clients are active.
