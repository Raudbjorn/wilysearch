//! Embedded Meilisearch engine core.
//!
//! Wraps the [`milli`] engine and provides an SDK-style interface for creating,
//! managing, and searching indexes directly within a Rust process.

pub mod error;
pub mod index;
pub mod meilisearch;
pub mod options;
pub mod preprocessing;
pub mod rag;
pub mod search;
pub mod settings;
pub mod vector;

pub use error::{Error, Result};
pub use index::{DocumentsResult, Index};
pub use meilisearch::{
    DumpInfo, ExperimentalFeatures, GlobalStats, HealthStatus, IndexInfo, IndexStats, Meilisearch,
    VersionInfo,
};
pub use options::MeilisearchOptions;
pub use preprocessing::{
    build_default_ttrpg_synonyms, CorrectionRecord, ExpandedQuery, ExpansionInfo, PipelineConfig,
    PreprocessingResult, ProcessedQuery, QueryPipeline, QueryPipelineBuilder, SynonymConfig,
    SynonymMap, SynonymType, TypoConfig, TypoCorrector,
};
pub use search::{
    ComputedFacets, FacetHit, FacetSearchQuery, FacetSearchResult, FacetStats,
    FederatedMultiSearchQuery, FederatedSearchResult, Federation, FederationOptions,
    GetDocumentsOptions, HitsInfo, HybridQuery, HybridSearchQuery, HybridSearchResult,
    MatchBounds, MatchingStrategy, MergeFacets, MultiSearchQuery, MultiSearchResult,
    SearchHit, SearchQuery, SearchResult, SearchResultWithIndex, SimilarQuery, SimilarResult,
};
pub use settings::{
    EmbedderSettings, EmbedderSource, FacetValuesSort, FacetingSettings, LocalizedAttributeRule,
    MinWordSizeForTypos, PaginationSettings, ProximityPrecision, Settings, TypoToleranceSettings,
};
pub use vector::{NoOpVectorStore, VectorStore};

#[cfg(feature = "surrealdb")]
pub use vector::{SurrealDbVectorStore, SurrealDbVectorStoreConfig};

pub use rag::{
    Embedder, Generator, RagPipeline, RagPipelineBuilder, RagResponse, Reranker,
    RetrievalQuery, RetrievalResult, Retriever, SearchType,
};
pub use rag::PipelineConfig as RagPipelineConfig;
