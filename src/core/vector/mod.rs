//! Vector store abstractions for Meilisearch.
//!
//! This module provides a trait-based abstraction for vector storage backends,
//! enabling pluggable implementations for different vector databases.

use anyhow::Result;
use roaring::RoaringBitmap;

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
}
