# Token Efficiency

Gatemini's context savings come from shape, not magic: a small fixed gateway surface, brief discovery responses by default, full schemas only on demand, and automatic output reduction on every `call_tool_chain` response.

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

### Three-tier search reducing wasted queries

`search_tools` uses a three-tier fallback — BM25 → trigram substring → fuzzy Levenshtein correction — so typos and partial terms still return the right tool without a follow-up query. The response also includes `try_also` IDF-scored distinctive terms for narrowing down follow-up searches.

### Automatic output processing in `call_tool_chain`

Every response from `call_tool_chain` passes through a pipeline of output reductions (all on by default, configurable via `output_config`):

| Reduction | What it does |
|-----------|-------------|
| Smart truncation | Preserves head 60% and tail 40% of output at line boundaries when size exceeds the limit |
| Auto-chunking | JSON responses over 10 KB are recursively decomposed into path-labeled chunks (e.g., `results > items > [0-4]`) at a 4 KB target chunk size |
| Uniform array collapse | Arrays where all items share the same key structure are collapsed: first 3 items shown in full, remaining items summarized by identity fields (`id`, `name`, `title`, `slug`, `key`, `label`) |
| Intent filtering | When the `intent` parameter is set and output exceeds 5 KB, lines are scored for relevance to the intent string and non-matching sections are suppressed |
| Response metadata | When any reduction occurs, the response includes a metadata header showing KB returned vs. KB processed and the savings ratio |

### Session stats via `gatemini://stats`

The `gatemini://stats` resource exposes per-session byte accounting:

- total bytes returned to context (after all reductions)
- total bytes processed (before reduction)
- per-tool savings breakdowns
- estimated reduction percentage

This lets you quantify how much context the output pipeline is saving in a live session without any external tooling.

## What is fixed versus variable

Fixed:

- the number of gateway meta-tools
- the existence of brief/full response modes
- the existence of resource templates
- the output reduction pipeline (smart truncation, auto-chunking, uniform array collapse, intent filtering)

Variable:

- total backend count
- total live tool count
- schema size per tool
- discovery depth per task
- effective savings ratio (depends on backend response sizes)

Because of that, any hard-coded token figure should be treated as an example or local measurement, not as a permanent truth about the repo.

## Practical measurement points

If you want to measure the real savings in your own config, compare:

1. the bytes returned by `search_tools` in brief mode versus full mode
2. the bytes returned by `tool_info` in brief mode versus full mode
3. the bytes for `gatemini://tools` versus serializing every registry entry with full schemas
4. the `gatemini://stats` resource before and after a representative task to see output-pipeline savings

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

`CallTracker` in `src/tracker.rs` records:

- recent tool calls
- per-tool usage counts
- per-backend latency percentiles (HDR histogram, p50/p95/p99)
- per-tool bytes returned (after reduction) and bytes processed (before reduction)
- session start time and total calls

The `record_bytes(tool, returned, processed)` method is called after every `call_tool_chain` output pass. `session_stats()` aggregates this into the `SessionStats` struct that backs `gatemini://stats`.
