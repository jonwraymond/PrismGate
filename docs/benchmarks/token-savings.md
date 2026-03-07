# Token Savings Benchmarks

This repository does not ship one canonical benchmark fixture for registry size, backend count, or schema mix. Treat this page as a measurement guide and a place to record local snapshots, not as a permanent source-of-truth count.

## What to benchmark

The most meaningful comparisons are:

1. `search_tools` brief versus full
2. `tool_info` brief versus full
3. `gatemini://tools` versus serializing all full schemas
4. fixed gateway-tool overhead versus direct exposure of every backend tool

## Suggested methodology

### Discovery payload size

For a chosen task:

1. call `search_tools` with `brief=true`
2. call `search_tools` with `brief=false`
3. compare response bytes or tokenized output

### Tool inspection payload size

For a chosen tool:

1. call `tool_info(detail="brief")`
2. call `tool_info(detail="full")`
3. compare response bytes or tokenized output

### Session-level overhead

Record:

- fixed gateway tool definitions
- server instruction block
- number of discovery steps needed before the first execution

## Example result template

```text
Registry snapshot date:
Backends loaded:
Tools loaded:

search_tools brief:
search_tools full:
tool_info brief:
tool_info full:
gatemini://tools:
all full schemas:
```

## Interpretation

You should expect the relative advantage of Gatemini to grow as:

- backend count grows
- tool count grows
- schema size grows

The gateway surface stays fixed while the naive "expose everything" surface expands with every backend you add.
