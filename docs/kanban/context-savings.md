# PrismGate — Context Savings & Token Efficiency Board

Epic: Enhance PrismGate token savings and context management
Target branch: `feat/context-savings`
Upstream repo: `jonwraymond/PrismGate`

## Quick Wins
- [x] 1. Wire telemetry: call `record_bytes` from `call_tool_chain` pipeline and expose via `gatemini://stats`
- [x] 2. Per-agent search flood guard: rolling window keyed by `session_id` for `search_tools` / `tool_info`
- [x] 3. Byte-budget truncation utilities: UTF-8-safe hard caps and JSON-aware truncation markers
- [x] 4. Session purge command: `gatemini purge` + optional MCP tool

## Medium-Lift Enhancements
- [x] 5. Persistent result handles: store raw output before destructive processing, return searchable handle + excerpts
- [x] 6. Structured retrieval: JSON-path / Markdown-heading / text-boundary chunks + ranked multi-query retrieval with budgets
- [x] 7. Sandbox data handles: extend bridge with `store/load/query` helpers for reuse across sandbox calls

## High-Lift / High-Reward
- [x] 8. FTS5-backed session event store with actor thread, `session_search`, handle binding, resume card, `session_note`
- [ ] 9. Host skills/hooks that auto-call `session_search(card=true)` after compact (Claude Code / Cursor / Codex / OpenCode)

## Cross-Cutting
- [x] A. Tests for telemetry, flood guard, truncation, result store, session store, resume card
- [x] B. Build verification: `cargo check/test/clippy/fmt` green on branch
- [x] C. Container image build verification via GHCR PR image path
- [ ] D. Docs update: README + `llms.txt` + `gatemini://call_tool_chain` guide
- [x] E. Open PR against upstream with changelog and rollback note
