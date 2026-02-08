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
    DumpInfo, ExperimentalFeatures, GlobalStats, HealthStatus, IndexInfo, IndexMetadata,
    IndexStats, Meilisearch, VersionInfo,
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
pub use vector::{InMemoryVectorStore, NoOpVectorStore, VectorStore};

#[cfg(feature = "surrealdb")]
pub use vector::{SurrealDbVectorStore, SurrealDbVectorStoreConfig};

pub use rag::{
    Embedder, Generator, RagPipeline, RagPipelineBuilder, RagResponse, Reranker,
    RetrievalQuery, RetrievalResult, Retriever, SearchType,
};
pub use rag::PipelineConfig as RagPipelineConfig;

/// Return the current UTC time as an RFC 3339 / ISO 8601 string.
///
/// Falls back to the Unix epoch if formatting fails (should never happen in
/// practice, but avoids a panic).
pub(crate) fn now_iso8601() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| {
            tracing::warn!("RFC3339 formatting of OffsetDateTime failed, using epoch fallback");
            "1970-01-01T00:00:00Z".to_string()
        })
}
