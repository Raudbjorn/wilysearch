//! Concrete implementations of RAG traits.
//!
//! This module provides ready-to-use implementations of the core RAG traits,
//! including retrievers, rerankers, and utility implementations.

use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::Arc;

use crate::core::rag::fusion::{fuse_retrieval_results, DEFAULT_RRF_K};
use crate::core::rag::traits::{Embedder, Generator, Reranker, Retriever};
use crate::core::rag::types::{RetrievalQuery, RetrievalResult, RetrievalSource, SearchType};
use crate::core::vector::VectorStore;
use crate::core::{Error, Result};

// =============================================================================
// Retriever Implementations
// =============================================================================

/// A retriever that uses a `VectorStore` for semantic search.
///
/// This retriever performs vector similarity search using an embedder
/// to convert queries into vectors.
///
/// # Type Parameters
///
/// * `E` - The embedder type
/// * `F` - The document fetcher closure type
/// * `D` - The document type
pub struct VectorStoreRetriever<E, F, D> {
    store: Arc<dyn VectorStore>,
    embedder: E,
    doc_fetcher: F,
    _phantom: PhantomData<D>,
}

impl<E, F, D> VectorStoreRetriever<E, F, D>
where
    E: Embedder,
    F: Fn(u32) -> Option<D> + Send + Sync,
    D: Send + Sync,
{
    /// Create a new vector store retriever.
    ///
    /// # Arguments
    ///
    /// * `store` - The vector store to search
    /// * `embedder` - The embedder for converting queries to vectors
    /// * `doc_fetcher` - A function to fetch full documents by ID
    pub fn new(store: Arc<dyn VectorStore>, embedder: E, doc_fetcher: F) -> Self {
        Self {
            store,
            embedder,
            doc_fetcher,
            _phantom: PhantomData,
        }
    }
}

impl<E, F, D> Retriever for VectorStoreRetriever<E, F, D>
where
    E: Embedder + Send + Sync,
    F: Fn(u32) -> Option<D> + Send + Sync,
    D: Clone + Send + Sync,
{
    type Document = D;

    async fn retrieve(&self, query: &RetrievalQuery) -> Result<Vec<RetrievalResult<D>>> {
        // Get or compute query vector
        let vector = match &query.vector {
            Some(v) => v.clone(),
            None => self.embedder.embed(&query.text).await?,
        };

        // Search the vector store
        let results = self
            .store
            .search(&vector, query.limit, None)
            .map_err(|e| Error::Internal(e.to_string()))?;

        // Fetch full documents and build results
        let mut retrieval_results = Vec::with_capacity(results.len());
        for (rank, (doc_id, score)) in results.into_iter().enumerate() {
            if let Some(doc) = (self.doc_fetcher)(doc_id) {
                retrieval_results.push(
                    RetrievalResult::new(doc, score, RetrievalSource::Semantic).with_rank(rank),
                );
            }
        }

        Ok(retrieval_results)
    }
}

/// A hybrid retriever that combines keyword and semantic search using RRF.
///
/// This retriever runs both a keyword retriever and a semantic retriever,
/// then fuses the results using Reciprocal Rank Fusion.
pub struct HybridRetriever<K, S, D> {
    keyword_retriever: K,
    semantic_retriever: S,
    k_constant: usize,
    _phantom: PhantomData<D>,
}

impl<K, S, D> HybridRetriever<K, S, D>
where
    K: Retriever<Document = D>,
    S: Retriever<Document = D>,
    D: Clone + Send + Sync,
{
    /// Create a new hybrid retriever.
    ///
    /// # Arguments
    ///
    /// * `keyword_retriever` - Retriever for keyword/BM25 search
    /// * `semantic_retriever` - Retriever for vector similarity search
    pub fn new(keyword_retriever: K, semantic_retriever: S) -> Self {
        Self {
            keyword_retriever,
            semantic_retriever,
            k_constant: DEFAULT_RRF_K,
            _phantom: PhantomData,
        }
    }

    /// Set the RRF k constant (default is 60).
    pub fn with_k_constant(mut self, k: usize) -> Self {
        self.k_constant = k;
        self
    }
}

impl<K, S, D> Retriever for HybridRetriever<K, S, D>
where
    K: Retriever<Document = D> + Send + Sync,
    S: Retriever<Document = D> + Send + Sync,
    D: Clone + Send + Sync + HasId,
{
    type Document = D;

    async fn retrieve(&self, query: &RetrievalQuery) -> Result<Vec<RetrievalResult<D>>> {
        let _semantic_ratio = query.search_type.semantic_ratio().unwrap_or(0.5);

        // Determine what to retrieve based on search type
        let (keyword_results, semantic_results) = match query.search_type {
            SearchType::Keyword => {
                let kw = self.keyword_retriever.retrieve(query).await?;
                (kw, Vec::new())
            }
            SearchType::Semantic => {
                let sem = self.semantic_retriever.retrieve(query).await?;
                (Vec::new(), sem)
            }
            SearchType::Hybrid { .. } => {
                // Run both retrievals concurrently
                let kw_query = RetrievalQuery {
                    search_type: SearchType::Keyword,
                    ..query.clone()
                };
                let sem_query = RetrievalQuery {
                    search_type: SearchType::Semantic,
                    ..query.clone()
                };

                // NOTE: These run sequentially. Using tokio::join! for concurrent
                // execution would require &self to be shared across futures, which
                // conflicts with the borrow checker without Arc wrapping. For most
                // use-cases the retrieval latency is dominated by I/O rather than
                // parallelism opportunity.
                let kw = self.keyword_retriever.retrieve(&kw_query).await?;
                let sem = self.semantic_retriever.retrieve(&sem_query).await?;
                (kw, sem)
            }
        };

        // If only one type has results, return those directly
        if keyword_results.is_empty() {
            return Ok(semantic_results);
        }
        if semantic_results.is_empty() {
            return Ok(keyword_results);
        }

        // Use RRF to fuse ranked lists. RRF is rank-based (score = 1/(k + rank)),
        // so pre-weighting the original scores before RRF is semantically wrong --
        // RRF replaces all scores with rank-derived values regardless of input scores.
        let fused = fuse_retrieval_results(
            vec![keyword_results, semantic_results],
            self.k_constant,
            query.limit,
            |doc| doc.id(),
        );

        Ok(fused)
    }
}

/// Trait for documents that have an ID for deduplication.
pub trait HasId {
    type Id: std::hash::Hash + Eq + Clone;
    fn id(&self) -> Self::Id;
}

impl HasId for String {
    type Id = String;
    fn id(&self) -> String {
        self.clone()
    }
}

impl HasId for serde_json::Value {
    type Id = String;
    fn id(&self) -> String {
        self.get("id")
            .or_else(|| self.get("_id"))
            .and_then(|v| match v {
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .unwrap_or_else(|| self.to_string())
    }
}

// =============================================================================
// Reranker Implementations
// =============================================================================

/// A simple reranker that just truncates results without reordering.
///
/// Use this as a baseline or when no reranking is needed.
pub struct TruncateReranker<D> {
    _phantom: PhantomData<D>,
}

impl<D> TruncateReranker<D> {
    /// Create a new truncate reranker.
    pub fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

impl<D> Default for TruncateReranker<D> {
    fn default() -> Self {
        Self::new()
    }
}

impl<D: Send + Sync> Reranker for TruncateReranker<D> {
    type Document = D;

    async fn rerank(
        &self,
        _query: &str,
        mut results: Vec<RetrievalResult<D>>,
        top_k: usize,
    ) -> Result<Vec<RetrievalResult<D>>> {
        results.truncate(top_k);

        // Update ranks and source
        for (rank, result) in results.iter_mut().enumerate() {
            result.rank = Some(rank);
            result.source = RetrievalSource::Reranked;
        }

        Ok(results)
    }
}

/// Placeholder for a cross-encoder based reranker.
///
/// This is a stub for future implementation. Cross-encoder rerankers
/// use a model to jointly encode the query and document, providing
/// more accurate relevance scores than bi-encoder approaches.
pub struct CrossEncoderReranker<D> {
    model_path: PathBuf,
    _phantom: PhantomData<D>,
}

impl<D> CrossEncoderReranker<D> {
    /// Create a new cross-encoder reranker.
    ///
    /// # Arguments
    ///
    /// * `model_path` - Path to the cross-encoder model
    pub fn new(model_path: impl Into<PathBuf>) -> Self {
        Self {
            model_path: model_path.into(),
            _phantom: PhantomData,
        }
    }

    /// Get the model path.
    pub fn model_path(&self) -> &PathBuf {
        &self.model_path
    }
}

impl<D: Send + Sync + Clone> Reranker for CrossEncoderReranker<D> {
    type Document = D;

    async fn rerank(
        &self,
        _query: &str,
        mut results: Vec<RetrievalResult<D>>,
        top_k: usize,
    ) -> Result<Vec<RetrievalResult<D>>> {
        // TODO: Implement actual cross-encoder reranking
        // For now, just truncate (same as TruncateReranker)

        log::warn!(
            "CrossEncoderReranker is not yet implemented, using truncation only. Model: {:?}",
            self.model_path
        );

        results.truncate(top_k);

        for (rank, result) in results.iter_mut().enumerate() {
            result.rank = Some(rank);
            result.source = RetrievalSource::Reranked;
        }

        Ok(results)
    }
}

// =============================================================================
// Generator Implementations
// =============================================================================

/// A stub generator that creates a simple template-based response.
///
/// This is useful for testing or when you want to bypass LLM generation.
pub struct TemplateGenerator {
    template: String,
}

impl TemplateGenerator {
    /// Create a new template generator.
    ///
    /// The template can include `{question}` and `{context}` placeholders.
    pub fn new(template: impl Into<String>) -> Self {
        Self {
            template: template.into(),
        }
    }

    /// Create a generator with a default template.
    pub fn default_template() -> Self {
        Self::new(
            "Based on the provided context, here is the answer to your question:\n\n\
             Question: {question}\n\n\
             Context:\n{context}\n\n\
             Answer: [Answer would be generated here by an LLM]",
        )
    }
}

impl Default for TemplateGenerator {
    fn default() -> Self {
        Self::default_template()
    }
}

impl Generator for TemplateGenerator {
    async fn generate(&self, prompt: &str, context: &[&str]) -> Result<String> {
        let context_str = context.join("\n\n---\n\n");

        let response = self
            .template
            .replace("{question}", prompt)
            .replace("{context}", &context_str);

        Ok(response)
    }

    fn model_name(&self) -> &str {
        "template-generator"
    }
}

/// A no-op generator that returns a fixed message.
///
/// Use this when you don't need generation but want to complete the pipeline.
pub struct NoOpGenerator;

impl Generator for NoOpGenerator {
    async fn generate(&self, _prompt: &str, _context: &[&str]) -> Result<String> {
        Ok("Generation not configured".to_string())
    }

    fn model_name(&self) -> &str {
        "no-op"
    }
}

// =============================================================================
// Embedder Implementations
// =============================================================================

/// A no-op embedder that returns zero vectors.
///
/// Use this for testing or when embeddings aren't needed.
pub struct NoOpEmbedder {
    dimensions: usize,
}

impl NoOpEmbedder {
    /// Create a new no-op embedder with the specified dimensions.
    pub fn new(dimensions: usize) -> Self {
        Self { dimensions }
    }
}

impl Embedder for NoOpEmbedder {
    async fn embed(&self, _text: &str) -> Result<Vec<f32>> {
        Ok(vec![0.0; self.dimensions])
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|_| vec![0.0; self.dimensions]).collect())
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn model_name(&self) -> &str {
        "no-op"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "surrealdb")]
    #[test]
    fn test_truncate_reranker() {
        let reranker = TruncateReranker::<String>::new();

        // Create test results
        let results = vec![
            RetrievalResult::new("doc1".to_string(), 0.9, RetrievalSource::Keyword),
            RetrievalResult::new("doc2".to_string(), 0.8, RetrievalSource::Keyword),
            RetrievalResult::new("doc3".to_string(), 0.7, RetrievalSource::Keyword),
        ];

        // Run reranker synchronously in test (tokio required)
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let reranked = rt.block_on(reranker.rerank("test", results, 2)).unwrap();

        assert_eq!(reranked.len(), 2);
        assert_eq!(reranked[0].document, "doc1");
        assert_eq!(reranked[1].document, "doc2");
        assert!(reranked
            .iter()
            .all(|r| r.source == RetrievalSource::Reranked));
    }

    #[cfg(feature = "surrealdb")]
    #[test]
    fn test_template_generator() {
        let generator = TemplateGenerator::new("Q: {question}\nA: Based on {context}");

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let response = rt
            .block_on(generator.generate("What is Rust?", &["Rust is a systems language"]))
            .unwrap();

        assert!(response.contains("What is Rust?"));
        assert!(response.contains("Rust is a systems language"));
    }

    #[cfg(feature = "surrealdb")]
    #[test]
    fn test_noop_embedder() {
        let embedder = NoOpEmbedder::new(384);

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let vector = rt.block_on(embedder.embed("test")).unwrap();

        assert_eq!(vector.len(), 384);
        assert!(vector.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn test_has_id_for_json() {
        let doc = serde_json::json!({"id": "test123", "content": "hello"});
        assert_eq!(doc.id(), "test123");

        let doc_without_id = serde_json::json!({"content": "hello"});
        assert!(!doc_without_id.id().is_empty());
    }
}
