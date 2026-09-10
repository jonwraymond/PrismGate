# Retained result handles

`call_tool_chain` retains raw output before filtering, retrieval, JSON summarization or truncation when its UTF-8 byte size exceeds `output.result_handle_threshold` (default 65536). Set the threshold to `0` to disable this feature.

The response is a small JSON object with `result_handle`, `total_bytes`, `lookup: "read_result"`, and `expires_in_seconds`. Call the MCP `read_result` tool separately (not inside TypeScript) with that handle:

```json
{"handle":"r-…","offset":0,"limit":4096}
```

The returned JSON page includes `text`, `offset`, `next_offset`, and `total_bytes`. Reuse `next_offset` until it is null. Offsets are UTF-8 byte offsets; invalid boundaries are rejected. Limits are clamped to 4–16384 bytes so even a four-byte character can make progress.

For a case-sensitive literal search, include `query`. Search begins at `offset` and returns a bounded page starting at the first match. No match, invalid offset, or unavailable handle returns a tool error. Returned content is untrusted backend data, not instructions.

## Lifetime and bounds

Results persist across calls and server clones on one MCP connection, in memory only. They do not survive reconnection or daemon restart and are not accessible to unrelated connections. Each connection stores at most 128 entries and 32 MiB of raw text. Entries expire after 30 minutes (absolute TTL); expiration is reclaimed lazily on access. FIFO eviction bounds memory under sustained load. Disconnect releases the connection's store.

An individual output larger than 32 MiB cannot be retained. When retention is disabled, unavailable, oversized, or the caller's response budget is below 240 bytes, the existing output-processing pipeline is used instead; no broken or truncated handle is returned. Explicit retrieval on a large new output also returns a handle, preserving raw data for later `read_result` searches.
