//! Composable RAG pipeline builder.
//!
//! This module provides a builder pattern for constructing RAG pipelines
//! from individual components (embedder, retriever, reranker, generator).
//!
//! # Example
//!
//! ```ignore
//! use wilysearch::core::rag::{RagPipelineBuilder, RagPipeline};
//!
//! let pipeline = RagPipelineBuilder::new()
//!     .with_embedder(my_embedder)
//!     .with_retriever(my_retriever)
//!     .with_reranker(my_reranker)
//!     .with_generator(my_generator)
//!     .build()?;
//!
//! let response = pipeline.query("What is Rust?").await?;
//! println!("Answer: {}", response.answer);
//! ```

use std::marker::PhantomData;
use std::time::Instant;

use crate::core::rag::traits::{Embedder, Generator, Reranker, Retriever};
use crate::core::rag::types::{
    RagResponse, RetrievalQuery, RetrievalResult, RetrievalStats, SearchType, SourceReference,
};
use crate::core::{Error, Result};

/// Configuration options for the RAG pipeline.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Number of documents to retrieve before reranking.
    pub retrieval_limit: usize,

    /// Number of documents to keep after reranking.
    pub rerank_limit: usize,

    /// Default search type for queries.
    pub default_search_type: SearchType,

    /// Maximum context length (in characters) to pass to the generator.
    pub max_context_chars: usize,

    /// System prompt template for generation.
    pub system_prompt: Option<String>,

    /// Whether to include source snippets in the response.
    pub include_snippets: bool,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            retrieval_limit: 20,
            rerank_limit: 5,
            default_search_type: SearchType::Hybrid { semantic_ratio: 0.5 },
            max_context_chars: 8000,
            system_prompt: None,
            include_snippets: true,
        }
    }
}

/// Builder for constructing RAG pipelines.
///
/// Use this to compose a pipeline from individual components.
/// At minimum, a retriever is required. Other components are optional:
///
/// - **Embedder**: Required for semantic/hybrid search if vectors aren't pre-computed
/// - **Retriever**: Required. Fetches relevant documents.
/// - **Reranker**: Optional. Improves result ordering.
/// - **Generator**: Required for full RAG (can be omitted for retrieval-only).
pub struct RagPipelineBuilder<E, R, Rr, G, D> {
    embedder: Option<E>,
    retriever: Option<R>,
    reranker: Option<Rr>,
    generator: Option<G>,
    config: PipelineConfig,
    _phantom: PhantomData<D>,
}

impl<D: Send + Sync + 'static> Default for RagPipelineBuilder<(), (), (), (), D> {
    fn default() -> Self {
        Self::new()
    }
}

impl<D: Send + Sync + 'static> RagPipelineBuilder<(), (), (), (), D> {
    /// Create a new pipeline builder with default configuration.
    pub fn new() -> Self {
        Self {
            embedder: None,
            retriever: None,
            reranker: None,
            generator: None,
            config: PipelineConfig::default(),
            _phantom: PhantomData,
        }
    }
}

impl<E, R, Rr, G, D> RagPipelineBuilder<E, R, Rr, G, D>
where
    D: Send + Sync + 'static,
{
    /// Set the embedder for converting text to vectors.
    pub fn with_embedder<E2>(self, embedder: E2) -> RagPipelineBuilder<E2, R, Rr, G, D>
    where
        E2: Embedder + 'static,
    {
        RagPipelineBuilder {
            embedder: Some(embedder),
            retriever: self.retriever,
            reranker: self.reranker,
            generator: self.generator,
            config: self.config,
            _phantom: PhantomData,
        }
    }

    /// Set the retriever for fetching relevant documents.
    pub fn with_retriever<R2>(self, retriever: R2) -> RagPipelineBuilder<E, R2, Rr, G, D>
    where
        R2: Retriever<Document = D> + 'static,
    {
        RagPipelineBuilder {
            embedder: self.embedder,
            retriever: Some(retriever),
            reranker: self.reranker,
            generator: self.generator,
            config: self.config,
            _phantom: PhantomData,
        }
    }

    /// Set the reranker for refining result ordering.
    pub fn with_reranker<Rr2>(self, reranker: Rr2) -> RagPipelineBuilder<E, R, Rr2, G, D>
    where
        Rr2: Reranker<Document = D> + 'static,
    {
        RagPipelineBuilder {
            embedder: self.embedder,
            retriever: self.retriever,
            reranker: Some(reranker),
            generator: self.generator,
            config: self.config,
            _phantom: PhantomData,
        }
    }

    /// Set the generator for producing answers.
    pub fn with_generator<G2>(self, generator: G2) -> RagPipelineBuilder<E, R, Rr, G2, D>
    where
        G2: Generator + 'static,
    {
        RagPipelineBuilder {
            embedder: self.embedder,
            retriever: self.retriever,
            reranker: self.reranker,
            generator: Some(generator),
            config: self.config,
            _phantom: PhantomData,
        }
    }

    /// Set the pipeline configuration.
    pub fn with_config(mut self, config: PipelineConfig) -> Self {
        self.config = config;
        self
    }

    /// Set the retrieval limit (documents to fetch before reranking).
    pub fn retrieval_limit(mut self, limit: usize) -> Self {
        self.config.retrieval_limit = limit;
        self
    }

    /// Set the rerank limit (documents to keep after reranking).
    pub fn rerank_limit(mut self, limit: usize) -> Self {
        self.config.rerank_limit = limit;
        self
    }

    /// Set the default search type.
    pub fn search_type(mut self, search_type: SearchType) -> Self {
        self.config.default_search_type = search_type;
        self
    }

    /// Set a custom system prompt for generation.
    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.config.system_prompt = Some(prompt.into());
        self
    }
}

impl<E, R, Rr, G, D> RagPipelineBuilder<E, R, Rr, G, D>
where
    E: Embedder + Send + Sync + 'static,
    R: Retriever<Document = D> + Send + Sync + 'static,
    Rr: Reranker<Document = D> + Send + Sync + 'static,
    G: Generator + Send + Sync + 'static,
    D: Send + Sync + 'static,
{
    /// Build the pipeline with all components.
    pub fn build(self) -> Result<RagPipeline<E, R, Rr, G, D>> {
        let retriever = self
            .retriever
            .ok_or_else(|| Error::Internal("RAG pipeline requires a retriever".to_string()))?;

        Ok(RagPipeline {
            embedder: self.embedder,
            retriever,
            reranker: self.reranker,
            generator: self.generator,
            config: self.config,
            _phantom: PhantomData,
        })
    }
}

// Specialized build for pipelines without all components
impl<R, D> RagPipelineBuilder<(), R, (), (), D>
where
    R: Retriever<Document = D> + Send + Sync + 'static,
    D: Send + Sync + 'static,
{
    /// Build a retrieval-only pipeline (no embedder, reranker, or generator).
    pub fn build_retrieval_only(self) -> Result<RagPipeline<(), R, (), (), D>> {
        let retriever = self
            .retriever
            .ok_or_else(|| Error::Internal("RAG pipeline requires a retriever".to_string()))?;

        Ok(RagPipeline {
            embedder: None,
            retriever,
            reranker: None,
            generator: None,
            config: self.config,
            _phantom: PhantomData,
        })
    }
}

/// A fully configured RAG pipeline.
///
/// The pipeline can be used for:
/// - Full RAG queries (retrieval + generation)
/// - Retrieval-only queries
/// - Reranking-only operations
pub struct RagPipeline<E, R, Rr, G, D> {
    embedder: Option<E>,
    retriever: R,
    reranker: Option<Rr>,
    generator: Option<G>,
    config: PipelineConfig,
    _phantom: PhantomData<D>,
}

impl<E, R, Rr, G, D> RagPipeline<E, R, Rr, G, D>
where
    E: Embedder + Send + Sync,
    R: Retriever<Document = D> + Send + Sync,
    Rr: Reranker<Document = D> + Send + Sync,
    G: Generator + Send + Sync,
    D: DocumentLike + Clone + Send + Sync,
{
    /// Execute the full RAG pipeline.
    ///
    /// This performs:
    /// 1. Query embedding (if needed for semantic search)
    /// 2. Document retrieval
    /// 3. Result reranking (if configured)
    /// 4. Answer generation (if generator is configured)
    ///
    /// # Arguments
    ///
    /// * `question` - The user's question
    ///
    /// # Returns
    ///
    /// A complete RAG response with answer, sources, and statistics.
    pub async fn query(&self, question: &str) -> Result<RagResponse> {
        let total_start = Instant::now();
        let mut stats = RetrievalStats::new();

        // 1. Embed query if needed
        let embed_start = Instant::now();
        let vector = if self.config.default_search_type.uses_semantic() {
            if let Some(embedder) = &self.embedder {
                Some(embedder.embed(question).await?)
            } else {
                None
            }
        } else {
            None
        };
        stats.embedding_time = embed_start.elapsed();

        // 2. Retrieve documents
        let retrieval_start = Instant::now();
        let query = RetrievalQuery {
            text: question.to_string(),
            vector,
            filter: None,
            limit: self.config.retrieval_limit,
            search_type: self.config.default_search_type,
            min_score: None,
            attributes_to_retrieve: None,
        };

        let mut results = self.retriever.retrieve(&query).await?;
        stats.total_retrieved = results.len();
        stats.retrieval_time = retrieval_start.elapsed();

        // 3. Rerank if configured
        let rerank_start = Instant::now();
        if let Some(reranker) = &self.reranker {
            results = reranker
                .rerank(question, results, self.config.rerank_limit)
                .await?;
        } else {
            results.truncate(self.config.rerank_limit);
        }
        stats.after_rerank = results.len();
        stats.rerank_time = rerank_start.elapsed();

        // 4. Generate answer if generator is configured
        let gen_start = Instant::now();
        let answer = if let Some(generator) = &self.generator {
            let context: Vec<String> = results
                .iter()
                .map(|r| r.document.to_context_string())
                .collect();
            let context_refs: Vec<&str> = context.iter().map(|s| s.as_str()).collect();

            generator.generate(question, &context_refs).await?
        } else {
            "No generator configured".to_string()
        };
        stats.generation_time = gen_start.elapsed();
        stats.total_time = total_start.elapsed();

        // Build source references
        let sources = results
            .iter()
            .map(|r| {
                let mut source = SourceReference::new(r.document.document_id(), r.score);
                if self.config.include_snippets {
                    source = source.with_snippet(r.document.snippet());
                }
                source
            })
            .collect();

        Ok(RagResponse::new(
            answer,
            sources,
            stats,
            question.to_string(),
        ))
    }
}

impl<E, R, Rr, G, D> RagPipeline<E, R, Rr, G, D>
where
    E: Embedder + Send + Sync,
    R: Retriever<Document = D> + Send + Sync,
    Rr: Reranker<Document = D> + Send + Sync,
    D: Clone + Send + Sync,
{
    /// Perform retrieval only (no generation).
    ///
    /// Useful when you want to get relevant documents without generating an answer.
    pub async fn retrieve(&self, question: &str) -> Result<Vec<RetrievalResult<D>>> {
        // Embed query if needed
        let vector = if self.config.default_search_type.uses_semantic() {
            if let Some(embedder) = &self.embedder {
                Some(embedder.embed(question).await?)
            } else {
                None
            }
        } else {
            None
        };

        // Build retrieval query
        let query = RetrievalQuery {
            text: question.to_string(),
            vector,
            filter: None,
            limit: self.config.retrieval_limit,
            search_type: self.config.default_search_type,
            min_score: None,
            attributes_to_retrieve: None,
        };

        // Retrieve
        let mut results = self.retriever.retrieve(&query).await?;

        // Rerank if configured
        if let Some(reranker) = &self.reranker {
            results = reranker
                .rerank(question, results, self.config.rerank_limit)
                .await?;
        } else {
            results.truncate(self.config.rerank_limit);
        }

        Ok(results)
    }

    /// Perform retrieval with a custom query.
    pub async fn retrieve_with_query(
        &self,
        query: &RetrievalQuery,
    ) -> Result<Vec<RetrievalResult<D>>> {
        let mut results = self.retriever.retrieve(query).await?;

        if let Some(reranker) = &self.reranker {
            results = reranker
                .rerank(&query.text, results, self.config.rerank_limit)
                .await?;
        }

        Ok(results)
    }

}

// Provide config and introspection for all pipeline types
impl<E, R, Rr, G, D> RagPipeline<E, R, Rr, G, D> {
    /// Check if the pipeline has an embedder configured.
    pub fn has_embedder(&self) -> bool {
        self.embedder.is_some()
    }

    /// Check if the pipeline has a generator configured.
    pub fn has_generator(&self) -> bool {
        self.generator.is_some()
    }

    /// Check if the pipeline has a reranker configured.
    pub fn has_reranker(&self) -> bool {
        self.reranker.is_some()
    }

    /// Get a reference to the pipeline configuration.
    pub fn config(&self) -> &PipelineConfig {
        &self.config
    }
}

/// Trait for documents that can be used in RAG pipelines.
///
/// Implement this trait to enable your document type to work with
/// the full RAG pipeline (including generation).
pub trait DocumentLike: Send + Sync {
    /// Get the document's unique identifier.
    fn document_id(&self) -> String;

    /// Get a text snippet suitable for display in sources.
    fn snippet(&self) -> String;

    /// Convert the document to a context string for the generator.
    fn to_context_string(&self) -> String;
}

// Implement DocumentLike for common types
impl DocumentLike for String {
    fn document_id(&self) -> String {
        // Hash the content for a pseudo-ID
        format!("{:x}", fxhash(self))
    }

    fn snippet(&self) -> String {
        if self.len() > 200 {
            // Find a char boundary at or before byte 200 to avoid panicking on multi-byte UTF-8
            let end = self.floor_char_boundary(200);
            format!("{}...", &self[..end])
        } else {
            self.clone()
        }
    }

    fn to_context_string(&self) -> String {
        self.clone()
    }
}

impl DocumentLike for serde_json::Value {
    fn document_id(&self) -> String {
        self.get("id")
            .or_else(|| self.get("_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{:x}", fxhash(&self.to_string())))
    }

    fn snippet(&self) -> String {
        // Try common content fields
        for field in ["content", "text", "body", "description", "summary"] {
            if let Some(text) = self.get(field).and_then(|v| v.as_str()) {
                if text.len() > 200 {
                    let end = text.floor_char_boundary(200);
                    return format!("{}...", &text[..end]);
                } else {
                    return text.to_string();
                }
            }
        }

        // Fall back to stringified JSON
        let s = self.to_string();
        if s.len() > 200 {
            let end = s.floor_char_boundary(200);
            format!("{}...", &s[..end])
        } else {
            s
        }
    }

    fn to_context_string(&self) -> String {
        // Try to extract meaningful text content
        for field in ["content", "text", "body"] {
            if let Some(text) = self.get(field).and_then(|v| v.as_str()) {
                return text.to_string();
            }
        }
        self.to_string()
    }
}

// Simple FxHash for generating pseudo-IDs
fn fxhash(s: &str) -> u64 {
    use std::ops::BitXor;
    const K: u64 = 0x517cc1b727220a95;
    let mut hash: u64 = 0;
    for byte in s.bytes() {
        hash = hash.rotate_left(5).bitxor(byte as u64).wrapping_mul(K);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::rag::types::RetrievalSource;

    // A simple test retriever
    struct MockRetriever;

    impl Retriever for MockRetriever {
        type Document = String;

        async fn retrieve(&self, query: &RetrievalQuery) -> Result<Vec<RetrievalResult<String>>> {
            Ok(vec![RetrievalResult::new(
                format!("Document about: {}", query.text),
                0.9,
                RetrievalSource::Keyword,
            )])
        }
    }

    #[test]
    fn test_pipeline_builder_requires_retriever() {
        // Can't call build without a retriever - this is a compile-time check
        // due to the type system, not a runtime check
    }

    #[test]
    fn test_pipeline_builder_with_retriever() {
        let result = RagPipelineBuilder::<(), (), (), (), String>::new()
            .with_retriever(MockRetriever)
            .build_retrieval_only();
        assert!(result.is_ok());
    }

    #[test]
    fn test_pipeline_config() {
        let pipeline = RagPipelineBuilder::<(), (), (), (), String>::new()
            .with_retriever(MockRetriever)
            .retrieval_limit(50)
            .rerank_limit(10)
            .build_retrieval_only()
            .unwrap();

        assert_eq!(pipeline.config().retrieval_limit, 50);
        assert_eq!(pipeline.config().rerank_limit, 10);
    }

    #[test]
    fn test_document_like_for_string() {
        let doc = "Hello, world!".to_string();
        assert!(!doc.document_id().is_empty());
        assert_eq!(doc.snippet(), "Hello, world!");
        assert_eq!(doc.to_context_string(), "Hello, world!");
    }

    #[test]
    fn test_document_like_for_json() {
        let doc = serde_json::json!({
            "id": "doc123",
            "content": "This is the document content.",
            "title": "Test Document"
        });

        assert_eq!(doc.document_id(), "doc123");
        assert_eq!(doc.snippet(), "This is the document content.");
        assert_eq!(doc.to_context_string(), "This is the document content.");
    }
}
