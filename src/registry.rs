use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

#[cfg(feature = "semantic")]
use crate::embeddings::EmbeddingIndex;

/// A tool entry in the registry, linking a tool to its backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolEntry {
    /// The tool name as exposed by the backend.
    pub name: String,
    /// Description from the backend's tool definition.
    pub description: String,
    /// The backend that owns this tool.
    pub backend_name: String,
    /// The full JSON schema for the tool's input.
    pub input_schema: Value,
}

/// Concurrent tool registry aggregating tools from all backends.
///
/// Uses DashMap for lock-free concurrent reads. Backends register
/// tools concurrently at startup without contention.
pub struct ToolRegistry {
    /// tool_name -> ToolEntry
    tools: DashMap<String, ToolEntry>,
    /// backend_name -> list of tool names
    backend_tools: DashMap<String, Vec<String>>,
    /// Optional semantic embedding index for hybrid search.
    #[cfg(feature = "semantic")]
    embedding_index: Option<EmbeddingIndex>,
}

impl ToolRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            tools: DashMap::new(),
            backend_tools: DashMap::new(),
            #[cfg(feature = "semantic")]
            embedding_index: None,
        })
    }

    /// Create a registry with an embedding index for hybrid BM25+semantic search.
    #[cfg(feature = "semantic")]
    pub fn new_with_embeddings(index: EmbeddingIndex) -> Arc<Self> {
        Arc::new(Self {
            tools: DashMap::new(),
            backend_tools: DashMap::new(),
            embedding_index: Some(index),
        })
    }

    /// Register tools discovered from a backend.
    pub fn register_backend_tools(&self, backend_name: &str, tools: Vec<ToolEntry>) {
        let tool_names: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();

        // Insert into DashMap first (source of truth for search results)
        #[cfg(feature = "semantic")]
        let tools_for_embedding: Vec<ToolEntry> = tools.to_vec();

        for tool in tools {
            self.tools.insert(tool.name.clone(), tool);
        }

        // Then update embeddings (any concurrent search will find valid tools)
        #[cfg(feature = "semantic")]
        if let Some(ref index) = self.embedding_index {
            index.add_tools(&tools_for_embedding);
        }

        self.backend_tools
            .insert(backend_name.to_string(), tool_names);
    }

    /// Remove all tools belonging to a backend.
    pub fn remove_backend_tools(&self, backend_name: &str) {
        if let Some((_, tool_names)) = self.backend_tools.remove(backend_name) {
            #[cfg(feature = "semantic")]
            if let Some(ref index) = self.embedding_index {
                index.remove_tools(&tool_names);
            }

            for name in tool_names {
                self.tools.remove(&name);
            }
        }
    }

    /// Get all tool entries.
    pub fn get_all(&self) -> Vec<ToolEntry> {
        self.tools.iter().map(|r| r.value().clone()).collect()
    }

    /// Get all tool names.
    pub fn get_all_names(&self) -> Vec<String> {
        self.tools.iter().map(|r| r.key().clone()).collect()
    }

    /// Look up a tool by exact name.
    pub fn get_by_name(&self, name: &str) -> Option<ToolEntry> {
        self.tools.get(name).map(|r| r.value().clone())
    }

    /// Get all tools for a specific backend.
    #[allow(dead_code)]
    pub fn get_by_backend(&self, backend_name: &str) -> Vec<ToolEntry> {
        if let Some(names) = self.backend_tools.get(backend_name) {
            names
                .iter()
                .filter_map(|n| self.tools.get(n).map(|r| r.value().clone()))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Search tools using BM25 ranking on name and description.
    ///
    /// BM25 parameters: k1=1.2 (term frequency saturation), b=0.75 (length normalization).
    /// Tool names are tokenized by splitting underscores/hyphens (e.g., "get_current_time"
    /// becomes ["get", "current", "time"]). Name tokens get a 2x boost over description tokens.
    pub fn search(&self, query: &str, limit: u32) -> Vec<ToolEntry> {
        let query_terms = tokenize(query);
        if query_terms.is_empty() {
            return Vec::new();
        }

        // Build corpus: collect (entry, doc_tokens) for all tools
        let corpus: Vec<(ToolEntry, Vec<String>)> = self
            .tools
            .iter()
            .map(|r| {
                let entry = r.value().clone();
                let mut tokens = tokenize(&entry.name);
                // Name tokens get 2x weight by appearing twice
                let name_tokens = tokens.clone();
                tokens.extend(name_tokens);
                tokens.extend(tokenize(&entry.description));
                (entry, tokens)
            })
            .collect();

        let n = corpus.len() as f64;
        if n == 0.0 {
            return Vec::new();
        }

        // Average document length
        let avgdl: f64 = corpus.iter().map(|(_, t)| t.len() as f64).sum::<f64>() / n;

        // Document frequency: how many docs contain each query term
        let mut df: HashMap<&str, f64> = HashMap::new();
        for term in &query_terms {
            let count = corpus
                .iter()
                .filter(|(_, tokens)| tokens.iter().any(|t| t == term))
                .count();
            df.insert(term.as_str(), count as f64);
        }

        // Score each document
        const K1: f64 = 1.2;
        const B: f64 = 0.75;

        let mut scored: Vec<(ToolEntry, f64)> = corpus
            .into_iter()
            .filter_map(|(entry, tokens)| {
                let dl = tokens.len() as f64;

                // Term frequencies in this doc
                let mut tf: HashMap<&str, f64> = HashMap::new();
                for term in &query_terms {
                    let count = tokens
                        .iter()
                        .filter(|t| t.as_str() == term.as_str())
                        .count();
                    tf.insert(term.as_str(), count as f64);
                }

                let mut score = 0.0f64;
                for term in &query_terms {
                    let term_freq = tf.get(term.as_str()).copied().unwrap_or(0.0);
                    if term_freq == 0.0 {
                        continue;
                    }
                    let doc_freq = df.get(term.as_str()).copied().unwrap_or(0.0);

                    // IDF: log((N - df + 0.5) / (df + 0.5) + 1)
                    let idf = ((n - doc_freq + 0.5) / (doc_freq + 0.5) + 1.0).ln();

                    // TF component with length normalization
                    let tf_norm =
                        (term_freq * (K1 + 1.0)) / (term_freq + K1 * (1.0 - B + B * dl / avgdl));

                    score += idf * tf_norm;
                }

                if score > 0.0 {
                    Some((entry, score))
                } else {
                    None
                }
            })
            .collect();

        // Sort by score descending, then by name for stability
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.name.cmp(&b.0.name))
        });

        scored.truncate(limit as usize);
        scored.into_iter().map(|(entry, _)| entry).collect()
    }

    /// Total number of registered tools.
    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    /// Number of registered backends.
    pub fn backend_count(&self) -> usize {
        self.backend_tools.len()
    }

    /// Get backend name for a given tool.
    #[allow(dead_code)]
    pub fn get_backend_for_tool(&self, tool_name: &str) -> Option<String> {
        self.tools
            .get(tool_name)
            .map(|r| r.value().backend_name.clone())
    }

    /// Get all backend names.
    #[allow(dead_code)]
    pub fn get_backend_names(&self) -> Vec<String> {
        self.backend_tools.iter().map(|r| r.key().clone()).collect()
    }

    /// Hybrid BM25 + semantic search using Reciprocal Rank Fusion (RRF).
    ///
    /// RRF combines ranked lists from different retrievers by scoring each result as
    /// `1 / (k + rank)` where k=60 is the standard IR constant. This normalizes
    /// the incomparable BM25 scores (0-15+) and cosine similarities (0-1) into a
    /// single ranking without hyperparameter tuning.
    #[cfg(feature = "semantic")]
    pub fn search_hybrid(&self, query: &str, limit: u32) -> Vec<ToolEntry> {
        let index = match &self.embedding_index {
            Some(idx) if !idx.is_empty() => idx,
            _ => return self.search(query, limit),
        };

        const RRF_K: f64 = 60.0;
        let fetch_limit = limit.max(30); // Fetch more candidates for fusion

        // Get BM25 ranked results
        let bm25_results = self.search(query, fetch_limit);

        // Get semantic ranked results
        let semantic_results = index.search(query, fetch_limit as usize);

        // RRF: accumulate 1/(k + rank) for each retriever
        let mut rrf_scores: HashMap<String, f64> = HashMap::new();

        for (rank, entry) in bm25_results.iter().enumerate() {
            *rrf_scores.entry(entry.name.clone()).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
        }

        for (rank, (name, _similarity)) in semantic_results.iter().enumerate() {
            *rrf_scores.entry(name.clone()).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
        }

        // Sort by combined RRF score descending
        let mut scored: Vec<(String, f64)> = rrf_scores.into_iter().collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        scored.truncate(limit as usize);

        // Look up full ToolEntry for each result
        scored
            .into_iter()
            .filter_map(|(name, _)| self.tools.get(&name).map(|r| r.value().clone()))
            .collect()
    }

    /// Export embedding vectors for cache persistence.
    #[cfg(feature = "semantic")]
    pub fn embedding_snapshot(&self) -> Option<HashMap<String, Vec<f32>>> {
        self.embedding_index.as_ref().map(|idx| idx.snapshot())
    }

    /// Restore embedding vectors from cache (avoids re-embedding on startup).
    #[cfg(feature = "semantic")]
    pub fn load_embeddings(&self, data: HashMap<String, Vec<f32>>) {
        if let Some(ref index) = self.embedding_index {
            index.load_snapshot(data);
        }
    }

    /// Export all tools grouped by backend name (for cache serialization).
    pub fn snapshot(&self) -> HashMap<String, Vec<ToolEntry>> {
        let mut result: HashMap<String, Vec<ToolEntry>> = HashMap::new();
        for entry in self.tools.iter() {
            result
                .entry(entry.value().backend_name.clone())
                .or_default()
                .push(entry.value().clone());
        }
        result
    }
}

/// Tokenize text into lowercase terms, splitting on non-alphanumeric characters.
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_entry(name: &str, desc: &str, backend: &str) -> ToolEntry {
        ToolEntry {
            name: name.to_string(),
            description: desc.to_string(),
            backend_name: backend.to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    #[test]
    fn test_register_and_get() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "exa",
            vec![
                make_entry("web_search", "Search the web", "exa"),
                make_entry("find_similar", "Find similar pages", "exa"),
            ],
        );

        assert_eq!(reg.tool_count(), 2);
        assert_eq!(reg.backend_count(), 1);

        let tool = reg.get_by_name("web_search").unwrap();
        assert_eq!(tool.backend_name, "exa");
        assert_eq!(tool.description, "Search the web");
    }

    #[test]
    fn test_remove_backend() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools("exa", vec![make_entry("web_search", "Search", "exa")]);
        reg.register_backend_tools(
            "tavily",
            vec![make_entry("tavily_search", "Search with Tavily", "tavily")],
        );

        assert_eq!(reg.tool_count(), 2);
        reg.remove_backend_tools("exa");
        assert_eq!(reg.tool_count(), 1);
        assert!(reg.get_by_name("web_search").is_none());
        assert!(reg.get_by_name("tavily_search").is_some());
    }

    #[test]
    fn test_search() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "exa",
            vec![
                make_entry("web_search", "Search the web using Exa", "exa"),
                make_entry("find_similar", "Find similar content", "exa"),
            ],
        );
        reg.register_backend_tools(
            "tavily",
            vec![make_entry(
                "tavily_search",
                "Web search via Tavily",
                "tavily",
            )],
        );

        // "search" appears in web_search (name+desc) and tavily_search (name+desc)
        // find_similar has no "search" token
        let results = reg.search("search", 10);
        assert_eq!(results.len(), 2);

        // "web" appears in web_search (name+desc) and tavily_search (desc)
        let results = reg.search("web", 10);
        assert_eq!(results.len(), 2);
        // web_search should rank higher (name match gets 2x boost)
        assert_eq!(results[0].name, "web_search");

        // "similar" only in find_similar
        let results = reg.search("similar", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "find_similar");
    }

    #[test]
    fn test_search_limit() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "test",
            vec![
                make_entry("tool_a", "A tool", "test"),
                make_entry("tool_b", "B tool", "test"),
                make_entry("tool_c", "C tool", "test"),
            ],
        );

        let results = reg.search("tool", 2);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_bm25_ranking_name_boost() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "backend",
            vec![
                make_entry("list_files", "List all files in a directory", "backend"),
                make_entry("search_code", "Search through code for patterns", "backend"),
                make_entry(
                    "delete_file",
                    "Delete a file from the filesystem",
                    "backend",
                ),
            ],
        );

        // "file" exact token: list_files has "files" (no match), delete_file has "file" (name match + desc match)
        // search_code desc has no "file" token
        let results = reg.search("file", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "delete_file");

        // "files" exact token: list_files has "files" in name and desc
        let results = reg.search("files", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "list_files");

        // Multi-term: "delete file" — delete_file matches both terms in name
        let results = reg.search("delete file", 10);
        assert_eq!(results[0].name, "delete_file");
    }

    #[test]
    fn test_bm25_multi_term_query() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "backend",
            vec![
                make_entry(
                    "get_current_time",
                    "Get current time in a specific timezone",
                    "backend",
                ),
                make_entry("convert_time", "Convert time between timezones", "backend"),
                make_entry(
                    "get_weather",
                    "Get current weather for a location",
                    "backend",
                ),
            ],
        );

        // Multi-term: "current time" should rank get_current_time highest (matches both terms)
        let results = reg.search("current time", 10);
        assert_eq!(results[0].name, "get_current_time");
    }

    #[test]
    fn test_bm25_no_match() {
        let reg = ToolRegistry::new();
        reg.register_backend_tools(
            "backend",
            vec![make_entry("web_search", "Search the web", "backend")],
        );

        let results = reg.search("database", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_tokenize() {
        assert_eq!(
            super::tokenize("get_current_time"),
            vec!["get", "current", "time"]
        );
        assert_eq!(
            super::tokenize("streamable-http"),
            vec!["streamable", "http"]
        );
        assert_eq!(
            super::tokenize("Search the WEB"),
            vec!["search", "the", "web"]
        );
    }
}
