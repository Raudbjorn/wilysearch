//! Vector store abstractions for Meilisearch.
//!
//! This module provides a trait-based abstraction for vector storage backends,
//! enabling pluggable implementations for different vector databases.

use anyhow::Result;
use roaring::RoaringBitmap;
use std::collections::HashMap;
use std::sync::RwLock;

#[cfg(feature = "surrealdb")]
pub mod surrealdb;

#[cfg(feature = "surrealdb")]
pub use self::surrealdb::{SurrealDbVectorStore, SurrealDbVectorStoreConfig};

/// A trait representing a vector store that can be used with Meilisearch.
///
/// This allows plugging in external vector databases or custom implementations.
pub trait VectorStore: Send + Sync {
    /// Add vectors for the given document IDs.
    ///
    /// If a document ID already exists, its vectors should be replaced.
    /// Each document can have multiple vectors.
    fn add_documents(&self, documents: &[(u32, Vec<Vec<f32>>)]) -> Result<()>;

    /// Remove vectors for the given document IDs.
    fn remove_documents(&self, ids: &[u32]) -> Result<()>;

    /// Perform a nearest neighbor search.
    ///
    /// - `vector`: The query vector.
    /// - `limit`: The maximum number of results to return.
    /// - `filter`: A bitmap of allowed document IDs (candidates).
    ///             If None, all documents are candidates.
    ///
    /// Returns a list of (document_id, distance/score).
    fn search(
        &self,
        vector: &[f32],
        limit: usize,
        filter: Option<&RoaringBitmap>,
    ) -> Result<Vec<(u32, f32)>>;

    /// Get the dimensionality of the vectors in this store.
    /// Returns None if the store is empty or dimension is unknown.
    fn dimensions(&self) -> Result<Option<usize>>;

    /// Remove all vectors from the store.
    fn clear(&self) -> Result<()>;
}

/// A dummy implementation of VectorStore that does nothing.
pub struct NoOpVectorStore;

impl VectorStore for NoOpVectorStore {
    fn add_documents(&self, _documents: &[(u32, Vec<Vec<f32>>)]) -> Result<()> {
        Ok(())
    }

    fn remove_documents(&self, _ids: &[u32]) -> Result<()> {
        Ok(())
    }

    fn search(
        &self,
        _vector: &[f32],
        _limit: usize,
        _filter: Option<&RoaringBitmap>,
    ) -> Result<Vec<(u32, f32)>> {
        Ok(Vec::new())
    }

    fn dimensions(&self) -> Result<Option<usize>> {
        Ok(None)
    }

    fn clear(&self) -> Result<()> {
        Ok(())
    }
}

/// An in-memory vector store backed by a `HashMap`.
///
/// Useful for testing and prototyping. Search uses brute-force cosine similarity.
pub struct InMemoryVectorStore {
    data: RwLock<HashMap<u32, Vec<Vec<f32>>>>,
}

impl InMemoryVectorStore {
    pub fn new() -> Self {
        Self {
            data: RwLock::new(HashMap::new()),
        }
    }

    /// Return a snapshot of all stored data for test inspection.
    pub fn snapshot(&self) -> HashMap<u32, Vec<Vec<f32>>> {
        self.data.read().unwrap().clone()
    }

    /// Return the number of documents in the store.
    pub fn len(&self) -> usize {
        self.data.read().unwrap().len()
    }

    /// Return true if the store is empty.
    pub fn is_empty(&self) -> bool {
        self.data.read().unwrap().is_empty()
    }
}

impl Default for InMemoryVectorStore {
    fn default() -> Self {
        Self::new()
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 { 0.0 } else { dot / denom }
}

impl VectorStore for InMemoryVectorStore {
    fn add_documents(&self, documents: &[(u32, Vec<Vec<f32>>)]) -> Result<()> {
        let mut data = self.data.write().unwrap();
        for (id, vectors) in documents {
            data.insert(*id, vectors.clone());
        }
        Ok(())
    }

    fn remove_documents(&self, ids: &[u32]) -> Result<()> {
        let mut data = self.data.write().unwrap();
        for id in ids {
            data.remove(id);
        }
        Ok(())
    }

    fn search(
        &self,
        vector: &[f32],
        limit: usize,
        filter: Option<&RoaringBitmap>,
    ) -> Result<Vec<(u32, f32)>> {
        let data = self.data.read().unwrap();
        let mut scored: Vec<(u32, f32)> = data
            .iter()
            .filter(|(id, _)| filter.map_or(true, |f| f.contains(**id)))
            .flat_map(|(id, vecs)| {
                vecs.iter()
                    .map(|v| (*id, cosine_similarity(vector, v)))
                    .collect::<Vec<_>>()
            })
            .collect();

        // Deduplicate by doc_id (keep best score)
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let mut seen = std::collections::HashSet::new();
        let deduped: Vec<(u32, f32)> = scored
            .into_iter()
            .filter(|(id, _)| seen.insert(*id))
            .take(limit)
            .collect();

        Ok(deduped)
    }

    fn dimensions(&self) -> Result<Option<usize>> {
        let data = self.data.read().unwrap();
        Ok(data
            .values()
            .flat_map(|vecs| vecs.first())
            .map(|v| v.len())
            .next())
    }

    fn clear(&self) -> Result<()> {
        self.data.write().unwrap().clear();
        Ok(())
    }
}
