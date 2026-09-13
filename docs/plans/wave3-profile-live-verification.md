# Wave 3 profile: live verification evidence

Status: partial acceptance; not approved for merge.

Worktree: `/home/hermes-exec/workspace/prismgate-wave3-profile`.
Baseline: `a9fe4f0`; verification exercised uncommitted changes.

## Executed

- `cargo build --quiet`: exit 0.
- Started the built `target/debug/prismgate` as a separate daemon process with an empty-backend YAML config, isolated HOME/cache/runtime directory, and private Unix socket.
- `prismgate profile --format json`: exit 0; parsed output was `{"scope":"process_since_reset","backends":[]}`.
- `prismgate profile`: exit 0; printed scope note and backend/calls/errors/percentile headings.
- `prismgate profile --backend missing`: exit 1; `no recorded calls for requested backend`.
- Sent SIGTERM to the test daemon and waited for its exit. Log confirmed daemon stopped.
- Harness exit: 0.

Evidence directory on execution host: `/tmp/pg-profile-smoke-fk8hhu8k` (temporary; includes config and daemon.log).
No production daemon or configuration was changed.

## Unresponsive daemon timeout: verified

Bound an isolated Unix socket without serving MCP responses, then invoked the built CLI with `profile --format json`. The CLI exited 1 after 15.01 seconds with `profile request timed out` / `deadline has elapsed`. Harness asserted nonzero exit, timeout message and completion below its 22-second safety limit; harness exit 0. No production socket was used.

## Outstanding gates

- Live populated-backend calls, backend failure accounting, filtering and reset tests. Unresponsive-daemon timeout is now verified above.
- Confirm shared and dedicated backend invocation accounting and MCP isError semantics.
- Complete user-facing docs and rerun full checks on final changes.
- Independent code review: two attempted reviewers failed HTTP 401 before inspecting code. Neither is an approval. Use a working alternative; do not silently substitute self-review.
- Separate feature PR, green CI, merge verification and release/build acceptance.
