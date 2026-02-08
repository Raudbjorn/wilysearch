//! Data types for RAG pipelines.
//!
//! This module contains the core data structures used throughout the RAG system,
//! including query representations, retrieval results, and response types.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Query for document retrieval.
///
/// This struct captures all the parameters needed to perform a retrieval operation,
/// supporting keyword, semantic, and hybrid search modes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalQuery {
    /// The text query string.
    pub text: String,

    /// Pre-computed query vector for semantic search.
    /// If None and semantic search is requested, the query will be embedded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector: Option<Vec<f32>>,

    /// Filter expression to narrow down results.
    /// Uses the backend's filter syntax.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,

    /// Maximum number of results to return.
    #[serde(default = "default_limit")]
    pub limit: usize,

    /// The type of search to perform.
    #[serde(default)]
    pub search_type: SearchType,

    /// Minimum score threshold for results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_score: Option<f32>,

    /// Specific attributes to retrieve from documents.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attributes_to_retrieve: Option<Vec<String>>,
}

fn default_limit() -> usize {
    10
}

impl RetrievalQuery {
    /// Create a new keyword-only retrieval query.
    pub fn keyword(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            vector: None,
            filter: None,
            limit: 10,
            search_type: SearchType::Keyword,
            min_score: None,
            attributes_to_retrieve: None,
        }
    }

    /// Create a new semantic-only retrieval query.
    pub fn semantic(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            vector: None,
            filter: None,
            limit: 10,
            search_type: SearchType::Semantic,
            min_score: None,
            attributes_to_retrieve: None,
        }
    }

    /// Create a new hybrid retrieval query with default ratio.
    pub fn hybrid(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            vector: None,
            filter: None,
            limit: 10,
            search_type: SearchType::Hybrid { semantic_ratio: 0.5 },
            min_score: None,
            attributes_to_retrieve: None,
        }
    }

    /// Set a pre-computed query vector.
    pub fn with_vector(mut self, vector: Vec<f32>) -> Self {
        self.vector = Some(vector);
        self
    }

    /// Set a filter expression.
    pub fn with_filter(mut self, filter: impl Into<String>) -> Self {
        self.filter = Some(filter.into());
        self
    }

    /// Set the maximum number of results.
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Set the semantic ratio for hybrid search.
    pub fn with_semantic_ratio(mut self, ratio: f32) -> Self {
        self.search_type = SearchType::Hybrid {
            semantic_ratio: ratio.clamp(0.0, 1.0),
        };
        self
    }

    /// Set the minimum score threshold.
    pub fn with_min_score(mut self, score: f32) -> Self {
        self.min_score = Some(score);
        self
    }

    /// Set attributes to retrieve.
    pub fn with_attributes(mut self, attrs: Vec<String>) -> Self {
        self.attributes_to_retrieve = Some(attrs);
        self
    }
}

/// The type of search to perform during retrieval.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SearchType {
    /// Traditional keyword/BM25 search.
    Keyword,

    /// Vector similarity search.
    Semantic,

    /// Combined keyword and semantic search.
    Hybrid {
        /// Ratio of semantic vs keyword scores (0.0 = pure keyword, 1.0 = pure semantic).
        semantic_ratio: f32,
    },
}

impl Default for SearchType {
    fn default() -> Self {
        Self::Hybrid { semantic_ratio: 0.5 }
    }
}

impl SearchType {
    /// Check if this search type uses keyword search.
    pub fn uses_keyword(&self) -> bool {
        matches!(self, SearchType::Keyword | SearchType::Hybrid { .. })
    }

    /// Check if this search type uses semantic search.
    pub fn uses_semantic(&self) -> bool {
        matches!(self, SearchType::Semantic | SearchType::Hybrid { .. })
    }

    /// Get the semantic ratio if applicable.
    pub fn semantic_ratio(&self) -> Option<f32> {
        match self {
            SearchType::Keyword => Some(0.0),
            SearchType::Semantic => Some(1.0),
            SearchType::Hybrid { semantic_ratio } => Some(*semantic_ratio),
        }
    }
}

/// A single result from the retrieval stage.
#[derive(Debug, Clone)]
pub struct RetrievalResult<D> {
    /// The retrieved document.
    pub document: D,

    /// The relevance score (higher is better).
    pub score: f32,

    /// The source of this result (which retrieval method produced it).
    pub source: RetrievalSource,

    /// Optional rank position in the original result set.
    pub rank: Option<usize>,
}

impl<D> RetrievalResult<D> {
    /// Create a new retrieval result.
    pub fn new(document: D, score: f32, source: RetrievalSource) -> Self {
        Self {
            document,
            score,
            source,
            rank: None,
        }
    }

    /// Set the rank position.
    pub fn with_rank(mut self, rank: usize) -> Self {
        self.rank = Some(rank);
        self
    }

    /// Map the document to a different type.
    pub fn map<U, F>(self, f: F) -> RetrievalResult<U>
    where
        F: FnOnce(D) -> U,
    {
        RetrievalResult {
            document: f(self.document),
            score: self.score,
            source: self.source,
            rank: self.rank,
        }
    }
}

/// The source/method that produced a retrieval result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalSource {
    /// Result from keyword/BM25 search.
    Keyword,

    /// Result from vector similarity search.
    Semantic,

    /// Result from hybrid search (fusion of keyword and semantic).
    Hybrid,

    /// Result that has been reranked.
    Reranked,

    /// Result from a custom source.
    Custom,
}

/// The complete response from a RAG pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagResponse {
    /// The generated answer.
    pub answer: String,

    /// References to source documents used to generate the answer.
    pub sources: Vec<SourceReference>,

    /// Statistics about the retrieval and generation process.
    pub stats: RetrievalStats,

    /// The original query that was asked.
    pub query: String,
}

impl RagResponse {
    /// Create a new RAG response.
    pub fn new(
        answer: String,
        sources: Vec<SourceReference>,
        stats: RetrievalStats,
        query: String,
    ) -> Self {
        Self {
            answer,
            sources,
            stats,
            query,
        }
    }
}

/// A reference to a source document used in the RAG response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceReference {
    /// The document's unique identifier.
    pub document_id: String,

    /// Optional chunk identifier within the document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_id: Option<String>,

    /// The relevance score of this source.
    pub relevance_score: f32,

    /// Optional snippet of text from the source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,

    /// Optional metadata about the source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl SourceReference {
    /// Create a new source reference.
    pub fn new(document_id: impl Into<String>, relevance_score: f32) -> Self {
        Self {
            document_id: document_id.into(),
            chunk_id: None,
            relevance_score,
            snippet: None,
            metadata: None,
        }
    }

    /// Add a chunk identifier.
    pub fn with_chunk(mut self, chunk_id: impl Into<String>) -> Self {
        self.chunk_id = Some(chunk_id.into());
        self
    }

    /// Add a text snippet.
    pub fn with_snippet(mut self, snippet: impl Into<String>) -> Self {
        self.snippet = Some(snippet.into());
        self
    }

    /// Add metadata.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

/// Statistics about the RAG pipeline execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RetrievalStats {
    /// Total documents retrieved before reranking.
    pub total_retrieved: usize,

    /// Documents remaining after reranking.
    pub after_rerank: usize,

    /// Time spent on embedding the query.
    #[serde(with = "duration_millis")]
    pub embedding_time: Duration,

    /// Time spent on retrieval.
    #[serde(with = "duration_millis")]
    pub retrieval_time: Duration,

    /// Time spent on reranking.
    #[serde(with = "duration_millis")]
    pub rerank_time: Duration,

    /// Time spent on generation.
    #[serde(with = "duration_millis")]
    pub generation_time: Duration,

    /// Total end-to-end time.
    #[serde(with = "duration_millis")]
    pub total_time: Duration,

    /// Number of tokens used in generation (if available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_used: Option<TokenUsage>,
}

impl RetrievalStats {
    /// Create empty stats.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set retrieval statistics.
    pub fn with_retrieval(mut self, total: usize, time: Duration) -> Self {
        self.total_retrieved = total;
        self.retrieval_time = time;
        self
    }

    /// Set reranking statistics.
    pub fn with_rerank(mut self, after_rerank: usize, time: Duration) -> Self {
        self.after_rerank = after_rerank;
        self.rerank_time = time;
        self
    }

    /// Set generation statistics.
    pub fn with_generation(mut self, time: Duration, tokens: Option<TokenUsage>) -> Self {
        self.generation_time = time;
        self.tokens_used = tokens;
        self
    }
}

/// Token usage statistics for generation.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Tokens in the prompt (context + question).
    pub prompt_tokens: usize,

    /// Tokens in the completion (generated answer).
    pub completion_tokens: usize,

    /// Total tokens used.
    pub total_tokens: usize,
}

impl TokenUsage {
    /// Create new token usage stats.
    pub fn new(prompt_tokens: usize, completion_tokens: usize) -> Self {
        Self {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
        }
    }
}

/// Serde helper for Duration as milliseconds.
mod duration_millis {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(duration.as_millis() as u64)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retrieval_query_builders() {
        let q = RetrievalQuery::keyword("test")
            .with_limit(20)
            .with_filter("category = 'books'");

        assert_eq!(q.text, "test");
        assert_eq!(q.limit, 20);
        assert_eq!(q.filter, Some("category = 'books'".to_string()));
        assert_eq!(q.search_type, SearchType::Keyword);
    }

    #[test]
    fn test_search_type_methods() {
        assert!(SearchType::Keyword.uses_keyword());
        assert!(!SearchType::Keyword.uses_semantic());

        assert!(!SearchType::Semantic.uses_keyword());
        assert!(SearchType::Semantic.uses_semantic());

        let hybrid = SearchType::Hybrid { semantic_ratio: 0.7 };
        assert!(hybrid.uses_keyword());
        assert!(hybrid.uses_semantic());
        assert_eq!(hybrid.semantic_ratio(), Some(0.7));
    }

    #[test]
    fn test_retrieval_result_map() {
        let result = RetrievalResult::new("hello".to_string(), 0.9, RetrievalSource::Keyword);
        let mapped = result.map(|s| s.len());

        assert_eq!(mapped.document, 5);
        assert_eq!(mapped.score, 0.9);
    }

    #[test]
    fn test_source_reference_builder() {
        let source = SourceReference::new("doc123", 0.95)
            .with_chunk("chunk_0")
            .with_snippet("relevant text here");

        assert_eq!(source.document_id, "doc123");
        assert_eq!(source.chunk_id, Some("chunk_0".to_string()));
        assert_eq!(source.snippet, Some("relevant text here".to_string()));
    }
}
