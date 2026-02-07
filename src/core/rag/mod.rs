//! Retrieval-Augmented Generation (RAG) pipeline system.
//!
//! This module provides a flexible, trait-based framework for building RAG pipelines
//! that can work with any search backend and generation model.
//!
//! # Architecture
//!
//! The RAG system is built around four core traits:
//!
//! - [`Embedder`]: Converts text into vector representations
//! - [`Retriever`]: Fetches relevant documents based on queries
//! - [`Reranker`]: Refines and reorders retrieval results
//! - [`Generator`]: Produces responses from retrieved context
//!
//! These traits can be composed into a [`RagPipeline`] using the [`RagPipelineBuilder`].
//!
//! # Quick Start
//!
//! ```ignore
//! use wilysearch::core::rag::{RagPipelineBuilder, SearchType};
//!
//! // Build a retrieval-only pipeline
//! let pipeline = RagPipelineBuilder::new()
//!     .with_retriever(my_retriever)
//!     .search_type(SearchType::Hybrid { semantic_ratio: 0.7 })
//!     .build_retrieval_only()?;
//!
//! // Retrieve relevant documents
//! let results = pipeline.retrieve("What is Rust?").await?;
//!
//! // Or build a full RAG pipeline with generation
//! let full_pipeline = RagPipelineBuilder::new()
//!     .with_embedder(my_embedder)
//!     .with_retriever(my_retriever)
//!     .with_reranker(my_reranker)
//!     .with_generator(my_generator)
//!     .build()?;
//!
//! let response = full_pipeline.query("What is Rust?").await?;
//! println!("Answer: {}", response.answer);
//! ```
//!
//! # Module Structure
//!
//! - [`traits`]: Core traits for RAG components
//! - [`types`]: Data structures for queries, results, and responses
//! - [`pipeline`]: Pipeline builder and executor
//! - [`fusion`]: Result fusion algorithms (RRF, weighted)
//! - [`implementations`]: Ready-to-use implementations
//!
//! # Hybrid Search with RRF
//!
//! The module includes Reciprocal Rank Fusion (RRF) for combining keyword
//! and semantic search results:
//!
//! ```
//! use wilysearch::core::rag::fusion::reciprocal_rank_fusion;
//!
//! let keyword_results = vec!["doc1", "doc2", "doc3"];
//! let semantic_results = vec!["doc2", "doc4", "doc1"];
//!
//! let fused = reciprocal_rank_fusion(
//!     &[keyword_results, semantic_results],
//!     60,  // RRF k constant
//!     10,  // limit
//!     |doc| *doc,
//! );
//! ```
//!
//! # Extensibility
//!
//! The trait-based design allows you to:
//!
//! - Use any embedding provider (OpenAI, Cohere, local models, etc.)
//! - Connect to any search backend (Meilisearch, Elasticsearch, custom)
//! - Implement custom reranking strategies
//! - Integrate with any LLM for generation
//!
//! # Example: Custom Embedder
//!
//! ```ignore
//! use wilysearch::core::rag::Embedder;
//! use wilysearch::core::Result;
//!
//! struct MyEmbedder {
//!     model: String,
//!     dimensions: usize,
//! }
//!
//! impl Embedder for MyEmbedder {
//!     async fn embed(&self, text: &str) -> Result<Vec<f32>> {
//!         // Call your embedding service
//!         todo!()
//!     }
//!
//!     async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
//!         // Batch embedding for efficiency
//!         todo!()
//!     }
//!
//!     fn dimensions(&self) -> usize {
//!         self.dimensions
//!     }
//! }
//! ```

pub mod fusion;
pub mod implementations;
pub mod pipeline;
pub mod traits;
pub mod types;

// Re-export commonly used types at the module level
pub use fusion::{fuse_retrieval_results, reciprocal_rank_fusion, weighted_score_fusion};
pub use implementations::{
    CrossEncoderReranker, HasId, HybridRetriever, NoOpEmbedder, NoOpGenerator, TemplateGenerator,
    TruncateReranker, VectorStoreRetriever,
};
pub use pipeline::{DocumentLike, PipelineConfig, RagPipeline, RagPipelineBuilder};
pub use traits::{
    Embedder, GenerationStream, Generator, PreprocessedQuery, QueryPreprocessor, Reranker,
    Retriever,
};
pub use types::{
    RagResponse, RetrievalQuery, RetrievalResult, RetrievalSource, RetrievalStats, SearchType,
    SourceReference, TokenUsage,
};
