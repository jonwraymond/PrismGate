# Token Efficiency

Gatemini's context savings come from shape, not magic: a small fixed gateway surface, brief discovery responses by default, and full schemas only on demand.

![Token comparison](diagrams/token-comparison.svg){ .diagram-wide }

## Where the savings come from

### Fixed gateway surface

Every session sees the same 7 gateway tools instead of every backend tool schema.

### Brief defaults

- `search_tools` defaults to `brief=true`
- `tool_info` defaults to `detail="brief"`

That means discovery usually starts with names, short descriptions, parameter names, and generated call examples instead of full JSON Schema blobs.

### On-demand schema loading

Agents only pull a full schema for tools they are likely to call.

### Resources as compact indexes

`gatemini://tools` provides a compressed inventory view without the cost of loading every full schema.

## What is fixed versus variable

Fixed:

- the number of gateway meta-tools
- the existence of brief/full response modes
- the existence of resource templates

Variable:

- total backend count
- total live tool count
- schema size per tool
- discovery depth per task

Because of that, any hard-coded token figure should be treated as an example or local measurement, not as a permanent truth about the repo.

## Practical measurement points

If you want to measure the real savings in your own config, compare:

1. the bytes returned by `search_tools` in brief mode versus full mode
2. the bytes returned by `tool_info` in brief mode versus full mode
3. the bytes for `gatemini://tools` versus serializing every registry entry with full schemas

## Cache and startup interaction

The cache system in `src/cache.rs` improves startup ergonomics:

- namespaced tools can be restored immediately from cache
- optional embeddings can be restored with them
- usage stats are restored into the tracker

Current details:

- cache version: `4`
- default path: platform cache directory plus `gatemini/cache.json`
- atomic writes: temp file plus rename

The old sibling-of-config cache path still exists in tests and migration helpers, but the normal runtime default is the platform cache directory.

## What the code already tracks

`CallTracker` already records:

- recent tool calls
- per-tool usage counts
- per-backend latency percentiles

That is useful context for future payload-size metrics, but the code does not yet emit direct token counts or response byte histograms as first-class telemetry.
