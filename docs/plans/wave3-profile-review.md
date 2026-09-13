# Wave 3 profile — failed review handoff

Baseline: a9fe4f0. Uncommitted implementation: /home/hermes-exec/workspace/prismgate-wave3-profile, branch feat/wave3-profile.
Independent read-only review: deleg_60a092af. Verdict: NOT READY. Original implementer interrupted; nothing shipped.

## Required remediation
- Replace nonexistent tools/call profile dispatch with a supported, tested daemon IPC contract. Reuse existing status IPC patterns; do not add a public tool just to serve the CLI.
- Remove EOF-based read_to_end on persistent sockets; bounded framed reads and timeouts are mandatory.
- Decode the actual protocol envelope, handle errors and validate response IDs.
- Register and implement prismgate://profile, using the same snapshot as the CLI.
- Enforce exact backend filtering; test missing backend behavior.
- Omit raw backend error strings from telemetry; retain only counts/classifications without payloads or credentials.
- Remove unrecorded outcome categories and unused timestamps rather than displaying misleading zero metrics.
- Clearly distinguish cumulative calls/errors from bounded latency sampling. Ensure reset semantics and finite empty-state metrics.
- Add CLI parsing, framed IPC, resource, filtering, privacy and live daemon round-trip tests, plus docs.

Review findings require validation against the current diff before fixing. Existing reviewer suggested adding a profile MCP tool, but the approved feature is CLI + resource: preserve that surface instead.

## Acceptance gates
cargo fmt --check; cargo clippy --all-targets --all-features -- -D warnings; cargo test; real daemon CLI and MCP resource smoke with bounded timeout. Capture actual exit codes. Fresh independent review after corrections, before push/merge. No previous review constitutes approval.
