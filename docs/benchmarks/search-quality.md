# Search Quality Benchmarks

This page documents the search behavior that exists in code today and suggests how to evaluate it against your own registry.

## Code-backed behavior

Implemented in `src/registry.rs`:

- BM25 keyword search
- optional semantic retrieval when the `semantic` feature is enabled
- Reciprocal Rank Fusion for hybrid ranking

Current fixed settings:

- BM25 `k1 = 1.2`
- BM25 `b = 0.75`
- name-token boost by duplication
- hybrid candidate pool fetches at least 30 items from each retriever before fusion

## What to test

Build a benchmark set with three categories:

1. exact-name or exact-term queries
2. conceptual queries that need semantic matching
3. ambiguous queries that need good ranking, not just recall

Suggested fields:

```text
query
expected tools
acceptable alternatives
notes
```

## Useful metrics

- MRR for "did the first useful tool rank high enough?"
- Recall@5 for "did discovery surface the right tool set?"
- nDCG@5 for "were the best tools near the top?"

## Scaling guidance

For the current implementation:

- BM25 is a scan over registry entries and short descriptions
- semantic lookup is brute-force cosine over stored vectors

That is a good fit for the current design space of dozens or hundreds of tools. If the registry grows into the many-thousands range, a more specialized indexing strategy may be worth revisiting.
