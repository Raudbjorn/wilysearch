//! Core RAG traits for building retrieval-augmented generation pipelines.
//!
//! This module defines the fundamental traits that enable backend-agnostic,
//! composable RAG pipelines. Each trait represents a distinct stage in the
//! RAG process:
//!
//! - [`Embedder`]: Converts text into vector representations
//! - [`Retriever`]: Fetches relevant documents based on queries
//! - [`Reranker`]: Refines and reorders retrieval results
//! - [`Generator`]: Produces responses from retrieved context

use crate::core::rag::types::{RetrievalQuery, RetrievalResult};
use crate::core::Result;

/// Embeds text into dense vector representations.
///
/// Implementations can wrap various embedding models (OpenAI, Cohere, local models, etc.)
/// to provide a unified interface for the RAG pipeline.
///
/// # Example
///
/// ```ignore
/// use wilysearch::core::rag::Embedder;
///
/// async fn embed_query(embedder: &dyn Embedder) {
///     let vector = embedder.embed("What is Rust?").await.unwrap();
///     assert_eq!(vector.len(), embedder.dimensions());
/// }
/// ```
pub trait Embedder: Send + Sync {
    /// Embed a single text into a vector.
    ///
    /// # Arguments
    ///
    /// * `text` - The text to embed
    ///
    /// # Returns
    ///
    /// A vector of floats representing the text embedding.
    fn embed(&self, text: &str) -> impl std::future::Future<Output = Result<Vec<f32>>> + Send;

    /// Embed multiple texts into vectors in a single batch.
    ///
    /// This can be more efficient than calling `embed` multiple times,
    /// as it allows batching API calls or parallel processing.
    ///
    /// # Arguments
    ///
    /// * `texts` - A slice of texts to embed
    ///
    /// # Returns
    ///
    /// A vector of vectors, one embedding per input text.
    fn embed_batch(
        &self,
        texts: &[&str],
    ) -> impl std::future::Future<Output = Result<Vec<Vec<f32>>>> + Send;

    /// Returns the dimensionality of the vectors produced by this embedder.
    ///
    /// This is useful for validating vector dimensions and pre-allocating storage.
    fn dimensions(&self) -> usize;

    /// Returns the model name/identifier for this embedder.
    ///
    /// Useful for logging and debugging.
    fn model_name(&self) -> &str {
        "unknown"
    }
}

/// Retrieves relevant documents based on a query.
///
/// This trait abstracts over different retrieval strategies (keyword, semantic, hybrid)
/// and different storage backends (Meilisearch, vector databases, etc.).
///
/// # Type Parameters
///
/// * `Document` - The type of documents returned by the retriever
pub trait Retriever: Send + Sync {
    /// The type of documents returned by this retriever.
    type Document: Send + Sync;

    /// Retrieve documents relevant to the given query.
    ///
    /// # Arguments
    ///
    /// * `query` - The retrieval query containing search parameters
    ///
    /// # Returns
    ///
    /// A vector of retrieval results, ordered by relevance (most relevant first).
    fn retrieve(
        &self,
        query: &RetrievalQuery,
    ) -> impl std::future::Future<Output = Result<Vec<RetrievalResult<Self::Document>>>> + Send;

    /// Retrieve documents and return the total count of matches.
    ///
    /// Some implementations may be able to provide an estimated or exact
    /// total count more efficiently.
    ///
    /// Default implementation calls `retrieve` and returns the length.
    fn retrieve_with_count(
        &self,
        query: &RetrievalQuery,
    ) -> impl std::future::Future<Output = Result<(Vec<RetrievalResult<Self::Document>>, usize)>> + Send
    {
        async move {
            let results = self.retrieve(query).await?;
            let count = results.len();
            Ok((results, count))
        }
    }
}

/// Reranks retrieval results to improve relevance ordering.
///
/// Rerankers typically use cross-encoder models or other techniques to
/// provide more accurate relevance scores than the initial retrieval stage.
///
/// # Type Parameters
///
/// * `Document` - The type of documents being reranked
pub trait Reranker: Send + Sync {
    /// The type of documents being reranked.
    type Document: Send + Sync;

    /// Rerank the given results based on relevance to the query.
    ///
    /// # Arguments
    ///
    /// * `query` - The original query string
    /// * `results` - The initial retrieval results to rerank
    /// * `top_k` - Maximum number of results to return after reranking
    ///
    /// # Returns
    ///
    /// Reranked results, ordered by new relevance scores (most relevant first).
    fn rerank(
        &self,
        query: &str,
        results: Vec<RetrievalResult<Self::Document>>,
        top_k: usize,
    ) -> impl std::future::Future<Output = Result<Vec<RetrievalResult<Self::Document>>>> + Send;
}

/// Generates responses from context.
///
/// This trait wraps language models (LLMs) to generate answers based on
/// retrieved context.
pub trait Generator: Send + Sync {
    /// Generate a response given a prompt and context documents.
    ///
    /// # Arguments
    ///
    /// * `prompt` - The user's question or prompt
    /// * `context` - Relevant text snippets from retrieved documents
    ///
    /// # Returns
    ///
    /// The generated response text.
    fn generate(
        &self,
        prompt: &str,
        context: &[&str],
    ) -> impl std::future::Future<Output = Result<String>> + Send;

    /// Generate a response with streaming output.
    ///
    /// Default implementation calls `generate` and returns the full response.
    fn generate_stream(
        &self,
        prompt: &str,
        context: &[&str],
    ) -> impl std::future::Future<Output = Result<GenerationStream>> + Send {
        async move {
            let response = self.generate(prompt, context).await?;
            Ok(GenerationStream::Complete(response))
        }
    }

    /// Returns the model name/identifier for this generator.
    fn model_name(&self) -> &str {
        "unknown"
    }

    /// Returns the maximum context length supported by this generator.
    fn max_context_length(&self) -> Option<usize> {
        None
    }
}

/// A stream of generated text tokens.
///
/// This is a simplified representation; in practice you'd likely use
/// a proper async stream type.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum GenerationStream {
    /// The complete response (for non-streaming generators).
    Complete(String),
    /// A token in the stream (for streaming generators).
    /// In practice, this would be an async stream.
    Token(String),
}

/// A query preprocessor that transforms queries before retrieval.
///
/// This can be used for query expansion, spelling correction,
/// intent classification, etc.
pub trait QueryPreprocessor: Send + Sync {
    /// Preprocess a query before retrieval.
    ///
    /// # Arguments
    ///
    /// * `query` - The original query string
    ///
    /// # Returns
    ///
    /// The preprocessed query (or queries for query expansion).
    fn preprocess(
        &self,
        query: &str,
    ) -> impl std::future::Future<Output = Result<PreprocessedQuery>> + Send;
}

/// The result of query preprocessing.
#[derive(Debug, Clone)]
pub struct PreprocessedQuery {
    /// The primary processed query.
    pub query: String,
    /// Additional expanded queries (for multi-query retrieval).
    pub expanded_queries: Vec<String>,
    /// Detected intent or classification.
    pub intent: Option<String>,
    /// Extracted entities or keywords.
    pub entities: Vec<String>,
}

impl PreprocessedQuery {
    /// Create a simple preprocessed query with just the main query.
    pub fn simple(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            expanded_queries: Vec::new(),
            intent: None,
            entities: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test that traits are object-safe where needed
    // Note: The async traits using impl Trait are not object-safe by design,
    // but we can use the dyn-compatible wrapper types in the pipeline module.

    #[test]
    fn test_preprocessed_query_simple() {
        let pq = PreprocessedQuery::simple("test query");
        assert_eq!(pq.query, "test query");
        assert!(pq.expanded_queries.is_empty());
        assert!(pq.intent.is_none());
        assert!(pq.entities.is_empty());
    }
}
