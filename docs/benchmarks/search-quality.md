# Search Quality Benchmarks

PrismGate's tool discovery uses BM25 keyword search and optional semantic search, combined via Reciprocal Rank Fusion (RRF). This document analyzes search quality, scaling behavior, and validation methodology.

## BM25 Parameters

PrismGate uses standard Okapi BM25 parameters validated by decades of IR research:

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| k1 | 1.2 | Term frequency saturation -- standard across [Lucene](https://lucene.apache.org/), [Elasticsearch](https://www.elastic.co/blog/practical-bm25-part-2-the-bm25-algorithm-and-its-variables), and [Tantivy](https://github.com/quickwit-oss/tantivy) |
| b | 0.75 | Document length normalization -- standard default; could be lowered for short tool descriptions |
| Name boost | 2x | Name tokens appear twice in document vector -- ensures exact name matches rank above description-only matches |

### Why These Parameters Work for Tool Search

Tool descriptions are short (10-200 words) and relatively uniform in length. This means:

- **k1=1.2**: Prevents long descriptions from dominating through raw term frequency. A tool mentioning "search" 5 times doesn't rank 5x higher than one mentioning it once.
- **b=0.75**: Provides moderate length normalization. Tools with longer descriptions aren't penalized too heavily, but very short descriptions get a slight boost.
- **2x name boost**: The most important signal for tool search is the tool name itself. `web_search` should rank above a tool that merely mentions "search" in its description.

For collections of very short, uniform documents, [Elasticsearch recommends](https://www.elastic.co/blog/practical-bm25-part-3-considerations-for-picking-b-and-k1-in-elasticsearch) potentially lowering b to 0.5-0.6, since length variation is minimal. This could be explored for PrismGate.

## Semantic Search Quality

### Model Choice: model2vec potion-base-8M

| Property | Value |
|----------|-------|
| Parameters | 8M |
| Embedding dimension | 256 |
| Size vs full transformers | 50x smaller |
| Speed vs full transformers | [500x faster on CPU](https://github.com/MinishLab/model2vec) |
| Method | Vocabulary distillation + PCA + Zipf weighting |

model2vec distills sentence transformer models by passing the vocabulary through the transformer, reducing dimensionality with PCA, and applying [Zipf weighting](https://pmc.ncbi.nlm.nih.gov/articles/PMC4176592/) to counteract high-frequency word bias. This produces static embeddings that can be looked up in O(1) per token rather than running transformer inference.

For tool discovery, the tradeoff is optimal: semantic understanding of "find information online" → "web search" doesn't require full transformer fidelity, but it does require sub-millisecond latency.

### Normalization

All vectors are L2-normalized in-place after encoding:

```rust
// L2 normalization: ||v|| = 1, so dot product = cosine similarity
let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
for x in v.iter_mut() { *x /= norm; }
```

With normalized vectors, cosine similarity reduces to dot product, which is [computationally cheaper](https://www.ibm.com/think/topics/cosine-similarity) (no division needed at query time).

## RRF Fusion Validation

### Score Scale Problem

BM25 and cosine similarity produce incomparable scores:

| Retriever | Score Range | Distribution |
|-----------|-------------|-------------|
| BM25 | 0 to ~15+ | Unbounded, IDF-dependent |
| Cosine similarity | -1 to 1 (0 to 1 with L2 norm) | Bounded |

Simple linear combination (e.g., `0.5*BM25 + 0.5*cosine`) requires careful weight tuning. RRF avoids this entirely by operating on rank positions:

```
RRF(d) = sum(1 / (K + rank_i(d))) for each retriever i
```

### Why K=60?

K=60 is the [standard constant from the original RRF paper](https://learn.microsoft.com/en-us/azure/search/hybrid-search-ranking), used in production by Azure AI Search, [OpenSearch](https://opensearch.org/blog/introducing-reciprocal-rank-fusion-hybrid-search/), and others. The constant controls how quickly scores decay with rank:

| Rank | Score (K=60) | Score (K=1) |
|------|-------------|-------------|
| 1 | 0.0164 | 0.5000 |
| 2 | 0.0161 | 0.3333 |
| 3 | 0.0159 | 0.2500 |
| 10 | 0.0143 | 0.0909 |
| 30 | 0.0111 | 0.0323 |

K=60 produces a gradual decay, giving weight to results ranked deep in each list. K=1 would heavily favor top-ranked results from each retriever.

### Candidate Pool

PrismGate fetches at least 30 candidates from each retriever before fusion:

```rust
let fetch_limit = limit.max(30);
```

This ensures the RRF has enough candidates from both retrievers to produce high-quality merged results, even when the final `limit` is small (e.g., 5).

## Scaling Analysis

### Brute-Force Cosine at Scale

At 258 tools, brute-force cosine similarity takes ~5 microseconds. How does this scale?

| Tool Count | Estimated Time | Approach |
|------------|---------------|----------|
| 258 | ~5 us | Brute-force (current) |
| 1,000 | ~20 us | Brute-force (sufficient) |
| 5,000 | ~100 us | Brute-force (still fast) |
| 10,000 | ~200 us | Consider HNSW index |
| 100,000 | ~2 ms | HNSW required |

With 256-dimensional vectors and modern CPUs utilizing SIMD, brute-force remains practical up to ~10,000 tools. Beyond that, approximate nearest neighbor (ANN) indices like HNSW become worthwhile.

PrismGate's current tool count (258) is well within the brute-force regime. Even scaling to 1,000 tools would add negligible latency.

### BM25 Scaling

BM25 scoring is O(n * q) where n = number of tools and q = number of query terms. With short queries (2-5 terms) and 258 tools, this is ~1,000 operations -- trivially fast. At 10,000 tools, it would be ~50,000 operations, still sub-millisecond.

For very large registries (100,000+ tools), an inverted index (as in [Tantivy](https://github.com/quickwit-oss/tantivy)) would be needed to avoid scanning all documents for each query.

## Evaluation Metrics

### Recommended Metrics

| Metric | Formula | Best For |
|--------|---------|----------|
| **MRR** (Mean Reciprocal Rank) | Average of 1/rank of first relevant result | Tool search (agent usually uses top result) |
| **nDCG@5** | Normalized Discounted Cumulative Gain at rank 5 | Comparing retrieval quality |
| **Recall@5** | Fraction of relevant tools in top 5 results | Ensuring relevant tools aren't missed |

MRR is the most appropriate primary metric for PrismGate because agents typically act on the first relevant tool found ([Galileo MRR guide](https://galileo.ai/blog/mrr-metric-ai-evaluation)).

### Proposed Test Harness

To systematically evaluate search quality:

```rust
struct SearchBenchmark {
    /// Natural language query
    query: String,
    /// Expected relevant tool names (ground truth)
    relevant_tools: Vec<String>,
}

let benchmarks = vec![
    SearchBenchmark {
        query: "search the web".to_string(),
        relevant_tools: vec!["web_search_exa", "tavily_search"],
    },
    SearchBenchmark {
        query: "find information online".to_string(),  // conceptual query
        relevant_tools: vec!["web_search_exa", "tavily_search"],
    },
    // ... 50+ test cases
];
```

Benchmark categories:

1. **Exact match**: Query terms appear in tool name (BM25 should excel)
2. **Conceptual match**: Synonyms, paraphrases (semantic should excel)
3. **Multi-term**: Complex queries combining multiple concepts (hybrid should excel)
4. **Ambiguous**: Queries matching multiple tools (ranking quality matters)

### Synthetic Registry Scaling Test

```
For registry_size in [100, 258, 500, 1000, 5000]:
  1. Generate synthetic tools (name + description)
  2. Insert into registry
  3. Run benchmark queries
  4. Measure: latency (P50/P95/P99), MRR, nDCG@5
  5. Compare BM25-only vs semantic-only vs hybrid
```

## Academic Context

PrismGate's search architecture is validated by recent academic work:

| Paper | Key Finding | Relevance |
|-------|-------------|-----------|
| [RAG-MCP (arXiv)](https://arxiv.org/abs/2505.03275) | RAG-MCP triples tool selection accuracy (43.13% vs 13.62% baseline) | Validates retrieval-first tool discovery |
| [Tool-to-Agent Retrieval](https://arxiv.org/abs/2511.01854) | 19.4% improvement in Recall@5 via unified embedding space | Validates semantic search for tools |
| [ToolLLM (ICLR 2024)](https://arxiv.org/abs/2307.16789) | 16,464 APIs organized with DFSDT planning | Large-scale tool discovery reference |
| [Hybrid search recall](https://medium.com/thinking-sand/hybrid-search-with-bm25-and-rank-fusion-for-accurate-results-456a70305dc5) | 53.4% passage recall vs BM25's 22.1% | Quantifies hybrid improvement |
| [PCA-RAG (arXiv)](https://arxiv.org/html/2504.08386v1) | PCA can reduce embeddings from 3,072 to 110 dims with moderate quality loss | Validates model2vec's PCA approach |

## Sources

- [`src/registry.rs`](../../src/registry.rs) -- BM25 and RRF implementation
- [`src/embeddings.rs`](../../src/embeddings.rs) -- Semantic embedding index
- [Okapi BM25 (Wikipedia)](https://en.wikipedia.org/wiki/Okapi_BM25) -- BM25 algorithm
- [Elastic BM25 Variables](https://www.elastic.co/blog/practical-bm25-part-2-the-bm25-algorithm-and-its-variables) -- Parameter tuning
- [Elastic BM25 Parameter Selection](https://www.elastic.co/blog/practical-bm25-part-3-considerations-for-picking-b-and-k1-in-elasticsearch) -- Short document considerations
- [Azure Hybrid Search RRF](https://learn.microsoft.com/en-us/azure/search/hybrid-search-ranking) -- RRF reference
- [OpenSearch RRF](https://opensearch.org/blog/introducing-reciprocal-rank-fusion-hybrid-search/) -- RRF implementation
- [Assembled RRF](https://www.assembled.com/blog/better-rag-results-with-reciprocal-rank-fusion-and-hybrid-search) -- Score normalization problem
- [Model2Vec](https://github.com/MinishLab/model2vec) -- Static embeddings
- [Model2Vec HuggingFace](https://huggingface.co/blog/Pringled/model2vec) -- potion model family
- [Tantivy](https://github.com/quickwit-oss/tantivy) -- Rust full-text search alternative
- [Weaviate Evaluation Metrics](https://weaviate.io/blog/retrieval-evaluation-metrics) -- IR metric guide
- [Pinecone Evaluation](https://www.pinecone.io/learn/offline-evaluation/) -- MRR, nDCG, MAP
- [Galileo MRR](https://galileo.ai/blog/mrr-metric-ai-evaluation) -- MRR for AI evaluation
