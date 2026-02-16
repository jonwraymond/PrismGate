use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

use model2vec_rs::model::StaticModel;
use tracing::{debug, info};

use crate::registry::ToolEntry;

/// Pre-computed embedding for a tool.
struct ToolEmbedding {
    /// L2-normalized embedding vector (dot product = cosine similarity).
    vector: Vec<f32>,
}

/// Manages embedding generation and brute-force vector search.
///
/// Thread-safe: `StaticModel` is `Send + Sync`, embeddings behind `RwLock`.
pub struct EmbeddingIndex {
    model: StaticModel,
    embeddings: RwLock<HashMap<String, ToolEmbedding>>,
}

impl EmbeddingIndex {
    /// Load an embedding model from a local path or HuggingFace Hub model ID.
    ///
    /// For HF hub models (e.g., "minishlab/potion-base-8M"), the model is
    /// auto-downloaded and cached locally on first use.
    ///
    /// Uses `hf_hub::ApiBuilder::from_env()` to respect `HF_HOME` for cache
    /// location, working around `model2vec_rs` using `Api::new()` which ignores it.
    pub fn new(model_path: &str) -> anyhow::Result<Self> {
        info!(model = model_path, "loading embedding model");

        // If model_path looks like a HF Hub ID (contains '/'), pre-download
        // the required files via the env-aware API so they land in the correct
        // cache directory (respecting HF_HOME). Then pass the local snapshot
        // path to from_pretrained.
        let local_path: Option<PathBuf> = if model_path.contains('/') && !std::path::Path::new(model_path).exists() {
            match Self::ensure_cached(model_path) {
                Ok(path) => Some(path),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to pre-cache model via hf-hub, falling back to model2vec default");
                    None
                }
            }
        } else {
            None
        };

        let load_path = local_path.as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| model_path.to_string());

        let model = StaticModel::from_pretrained(&load_path, None, Some(true), None)
            .map_err(|e| anyhow::anyhow!("failed to load embedding model: {e}"))?;
        info!(model = model_path, "embedding model loaded");
        Ok(Self {
            model,
            embeddings: RwLock::new(HashMap::new()),
        })
    }

    /// Download model files using the env-aware `hf_hub` API.
    ///
    /// Returns the local snapshot directory path containing the model files.
    /// Respects `HF_HOME` env var for cache location.
    fn ensure_cached(model_id: &str) -> anyhow::Result<PathBuf> {
        use hf_hub::api::sync::ApiBuilder;

        let api = ApiBuilder::from_env()
            .build()
            .map_err(|e| anyhow::anyhow!("hf-hub API init failed: {e}"))?;
        let repo = api.model(model_id.to_string());

        // Download the three files model2vec needs
        let _tok = repo.get("tokenizer.json")
            .map_err(|e| anyhow::anyhow!("failed to download tokenizer.json: {e}"))?;
        let _mdl = repo.get("model.safetensors")
            .map_err(|e| anyhow::anyhow!("failed to download model.safetensors: {e}"))?;
        let cfg = repo.get("config.json")
            .map_err(|e| anyhow::anyhow!("failed to download config.json: {e}"))?;

        // Return the parent directory (snapshot dir) as the local model path
        let snapshot_dir = cfg.parent()
            .ok_or_else(|| anyhow::anyhow!("config.json has no parent directory"))?;

        info!(path = %snapshot_dir.display(), "model cached via hf-hub");
        Ok(snapshot_dir.to_path_buf())
    }

    /// Embed text and L2-normalize the result.
    ///
    /// Normalizing means dot product equals cosine similarity,
    /// avoiding the division in the search hot path.
    fn embed_text(&self, text: &str) -> Vec<f32> {
        let mut v = self.model.encode_single(text);
        l2_normalize(&mut v);
        v
    }

    /// Add embeddings for a batch of tools.
    ///
    /// Uses batch encoding for efficiency. Each tool's embedding text is
    /// `"{name} {description}"` to capture both identity and semantics.
    pub fn add_tools(&self, tools: &[ToolEntry]) {
        if tools.is_empty() {
            return;
        }

        let texts: Vec<String> = tools
            .iter()
            .map(|t| format!("{} {}", t.name, t.description))
            .collect();

        let mut vectors = self.model.encode(&texts);

        // Normalize all vectors before acquiring the lock
        for vec in &mut vectors {
            l2_normalize(vec);
        }

        let mut store = self.embeddings.write().expect("embedding lock poisoned");
        for (tool, vec) in tools.iter().zip(vectors) {
            store.insert(tool.name.clone(), ToolEmbedding { vector: vec });
        }

        debug!(count = tools.len(), "embedded tools");
    }

    /// Remove embeddings for tools belonging to a deregistered backend.
    pub fn remove_tools(&self, tool_names: &[String]) {
        let mut store = self.embeddings.write().expect("embedding lock poisoned");
        for name in tool_names {
            store.remove(name);
        }
    }

    /// Brute-force cosine similarity search.
    ///
    /// For ~258 vectors at 256 dimensions, this completes in ~5 microseconds.
    /// HNSW overhead only pays off at 10K+ vectors.
    pub fn search(&self, query: &str, limit: usize) -> Vec<(String, f32)> {
        let query_vec = self.embed_text(query);

        let store = self.embeddings.read().expect("embedding lock poisoned");
        let mut scored: Vec<(String, f32)> = store
            .iter()
            .map(|(name, emb)| {
                let similarity = dot_product(&query_vec, &emb.vector);
                (name.clone(), similarity)
            })
            .collect();

        // Sort descending by similarity
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        scored
    }

    /// Export all embeddings for cache persistence.
    pub fn snapshot(&self) -> HashMap<String, Vec<f32>> {
        let store = self.embeddings.read().expect("embedding lock poisoned");
        store
            .iter()
            .map(|(name, emb)| (name.clone(), emb.vector.clone()))
            .collect()
    }

    /// Restore embeddings from a cached snapshot (skips re-embedding).
    pub fn load_snapshot(&self, data: HashMap<String, Vec<f32>>) {
        let mut store = self.embeddings.write().expect("embedding lock poisoned");
        for (name, vector) in data {
            store.insert(name, ToolEmbedding { vector });
        }
        info!(count = store.len(), "loaded embeddings from cache");
    }

    /// Number of currently stored embeddings.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.embeddings
            .read()
            .expect("embedding lock poisoned")
            .len()
    }

    /// Whether the embedding store is empty.
    pub fn is_empty(&self) -> bool {
        self.embeddings
            .read()
            .expect("embedding lock poisoned")
            .is_empty()
    }
}

/// L2-normalize a vector in-place.
fn l2_normalize(vec: &mut [f32]) {
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in vec.iter_mut() {
            *x /= norm;
        }
    }
}

/// Dot product of two vectors (equals cosine similarity when both are L2-normalized).
fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_l2_normalize() {
        let mut v = vec![3.0, 4.0];
        l2_normalize(&mut v);
        assert!((v[0] - 0.6).abs() < 1e-6);
        assert!((v[1] - 0.8).abs() < 1e-6);

        // Norm should be ~1.0
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_l2_normalize_zero_vector() {
        let mut v = vec![0.0, 0.0, 0.0];
        l2_normalize(&mut v);
        assert!(v.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn test_dot_product() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((dot_product(&a, &b) - 1.0).abs() < 1e-6);

        let c = vec![0.0, 1.0, 0.0];
        assert!(dot_product(&a, &c).abs() < 1e-6);
    }

    #[test]
    fn test_dot_product_normalized_is_cosine() {
        let mut a = vec![3.0, 4.0];
        let mut b = vec![4.0, 3.0];
        l2_normalize(&mut a);
        l2_normalize(&mut b);

        let cosine = dot_product(&a, &b);
        // cos(angle between [3,4] and [4,3]) = (12+12)/(5*5) = 24/25 = 0.96
        assert!((cosine - 0.96).abs() < 1e-6);
    }
}
