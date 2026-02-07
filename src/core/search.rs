//! Search types for meilisearch-lib.
//!
//! Provides query, result, and hit types that match the Meilisearch HTTP API
//! shape using `serde(rename_all = "camelCase")` for JSON compatibility.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use indexmap::IndexMap;

// ============================================================================
// SearchQuery
// ============================================================================

/// Query parameters for search requests.
///
/// Matches the Meilisearch HTTP API search body (minus server-only fields).
/// All fields use camelCase serialization for API shape parity.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    /// The search query string. If `None`, matches all documents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,

    /// Pre-computed query vector for pure vector search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector: Option<Vec<f32>>,

    /// Hybrid search configuration (combines keyword + semantic).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hybrid: Option<HybridQuery>,

    /// Number of results to skip. Default: 0.
    #[serde(default)]
    pub offset: usize,

    /// Maximum number of results to return. Default: 20.
    #[serde(default = "default_limit")]
    pub limit: usize,

    /// Request a specific page (switches to page-based pagination).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,

    /// Hits per page (switches to page-based pagination).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hits_per_page: Option<usize>,

    /// Attributes to include in returned documents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes_to_retrieve: Option<BTreeSet<String>>,

    /// Whether to include embedding vectors in returned documents.
    #[serde(default)]
    pub retrieve_vectors: bool,

    /// Attributes to highlight with matching terms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes_to_highlight: Option<HashSet<String>>,

    /// Tag inserted before a highlighted term. Default: `<em>`.
    #[serde(default = "default_highlight_pre_tag")]
    pub highlight_pre_tag: String,

    /// Tag inserted after a highlighted term. Default: `</em>`.
    #[serde(default = "default_highlight_post_tag")]
    pub highlight_post_tag: String,

    /// Attributes whose values are cropped around matching terms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes_to_crop: Option<Vec<String>>,

    /// Maximum length of cropped values in words. Default: 10.
    #[serde(default = "default_crop_length")]
    pub crop_length: usize,

    /// Marker string for cropped boundaries. Default: `"..."`.
    #[serde(default = "default_crop_marker")]
    pub crop_marker: String,

    /// Include `_rankingScore` in each hit.
    #[serde(default)]
    pub show_ranking_score: bool,

    /// Include `_rankingScoreDetails` breakdown in each hit.
    #[serde(default)]
    pub show_ranking_score_details: bool,

    /// Include `_matchesPosition` in each hit.
    #[serde(default)]
    pub show_matches_position: bool,

    /// Minimum ranking score (0.0-1.0). Hits below this are excluded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranking_score_threshold: Option<f64>,

    /// Filter expression. Supports string or array-of-arrays syntax.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Value>,

    /// Sort expressions, e.g. `["price:asc", "rating:desc"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<Vec<String>>,

    /// Return only documents with distinct values for this attribute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distinct: Option<String>,

    /// Facet attributes to compute distribution for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facets: Option<Vec<String>>,

    /// Strategy for matching query terms.
    #[serde(default)]
    pub matching_strategy: MatchingStrategy,

    /// Restrict search to these attributes only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes_to_search_on: Option<Vec<String>>,

    /// Locales for language-specific tokenization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locales: Option<Vec<String>>,
}

fn default_limit() -> usize { 20 }
fn default_crop_length() -> usize { 10 }
fn default_crop_marker() -> String { "...".to_string() }
fn default_highlight_pre_tag() -> String { "<em>".to_string() }
fn default_highlight_post_tag() -> String { "</em>".to_string() }

impl SearchQuery {
    /// Create a new search query with the given query string.
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            q: Some(query.into()),
            limit: 20,
            crop_length: 10,
            crop_marker: "...".to_string(),
            highlight_pre_tag: "<em>".to_string(),
            highlight_post_tag: "</em>".to_string(),
            ..Default::default()
        }
    }

    /// Create an empty search query that matches all documents.
    pub fn match_all() -> Self {
        Self::default()
    }

    /// Set the maximum number of results to return.
    pub fn with_limit(mut self, limit: usize) -> Self { self.limit = limit; self }
    /// Set the number of results to skip.
    pub fn with_offset(mut self, offset: usize) -> Self { self.offset = offset; self }
    /// Switch to page-based pagination and set the page number (1-indexed).
    pub fn with_page(mut self, page: usize) -> Self { self.page = Some(page); self }
    /// Set the number of hits per page (enables page-based pagination).
    pub fn with_hits_per_page(mut self, hpp: usize) -> Self { self.hits_per_page = Some(hpp); self }
    /// Set a filter expression (string or JSON array-of-arrays syntax).
    pub fn with_filter(mut self, filter: impl Into<Value>) -> Self { self.filter = Some(filter.into()); self }
    /// Set sort criteria, e.g. `["price:asc", "rating:desc"]`.
    pub fn with_sort(mut self, sort: Vec<String>) -> Self { self.sort = Some(sort); self }
    /// Request facet distribution for the given attributes.
    pub fn with_facets(mut self, facets: Vec<String>) -> Self { self.facets = Some(facets); self }
    /// Restrict which document attributes are included in the response.
    pub fn with_attributes_to_retrieve(mut self, attrs: impl IntoIterator<Item = String>) -> Self {
        self.attributes_to_retrieve = Some(attrs.into_iter().collect());
        self
    }
    /// Set attributes to highlight with matching terms.
    pub fn with_attributes_to_highlight(mut self, attrs: impl IntoIterator<Item = String>) -> Self {
        self.attributes_to_highlight = Some(attrs.into_iter().collect());
        self
    }
    /// Set attributes to crop around matching terms.
    pub fn with_attributes_to_crop(mut self, attrs: Vec<String>) -> Self { self.attributes_to_crop = Some(attrs); self }
    /// Set the maximum length (in words) for cropped values.
    pub fn with_crop_length(mut self, len: usize) -> Self { self.crop_length = len; self }
    /// Set the marker string for cropped boundaries (default: `"..."`).
    pub fn with_crop_marker(mut self, marker: impl Into<String>) -> Self { self.crop_marker = marker.into(); self }
    /// Set the tag inserted before highlighted terms (default: `<em>`).
    pub fn with_highlight_pre_tag(mut self, tag: impl Into<String>) -> Self { self.highlight_pre_tag = tag.into(); self }
    /// Set the tag inserted after highlighted terms (default: `</em>`).
    pub fn with_highlight_post_tag(mut self, tag: impl Into<String>) -> Self { self.highlight_post_tag = tag.into(); self }
    /// Include `_rankingScore` in each hit.
    pub fn with_ranking_score(mut self, show: bool) -> Self { self.show_ranking_score = show; self }
    /// Include `_rankingScoreDetails` breakdown in each hit.
    pub fn with_ranking_score_details(mut self, show: bool) -> Self { self.show_ranking_score_details = show; self }
    /// Include `_matchesPosition` in each hit.
    pub fn with_matches_position(mut self, show: bool) -> Self { self.show_matches_position = show; self }
    /// Set the strategy for matching query terms.
    pub fn with_matching_strategy(mut self, strategy: MatchingStrategy) -> Self { self.matching_strategy = strategy; self }
    /// Set a minimum ranking score threshold; hits below this are excluded.
    pub fn with_ranking_score_threshold(mut self, threshold: f64) -> Self { self.ranking_score_threshold = Some(threshold); self }
    /// Return only documents with distinct values for this attribute.
    pub fn with_distinct(mut self, attr: impl Into<String>) -> Self { self.distinct = Some(attr.into()); self }
    /// Set locales for language-specific tokenization.
    pub fn with_locales(mut self, locales: Vec<String>) -> Self { self.locales = Some(locales); self }
    /// Restrict search to specific attributes.
    pub fn with_attributes_to_search_on(mut self, attrs: Vec<String>) -> Self { self.attributes_to_search_on = Some(attrs); self }
    /// Set a pre-computed query vector for pure vector search.
    pub fn with_vector(mut self, vector: Vec<f32>) -> Self { self.vector = Some(vector); self }
    /// Enable hybrid (keyword + semantic) search with the given configuration.
    pub fn with_hybrid(mut self, hybrid: HybridQuery) -> Self { self.hybrid = Some(hybrid); self }
    /// Include embedding vectors in returned documents.
    pub fn with_retrieve_vectors(mut self, retrieve: bool) -> Self { self.retrieve_vectors = retrieve; self }

    /// Helper to get query string (for backward compat).
    pub fn query_str(&self) -> Option<&str> {
        self.q.as_deref()
    }
}

// ============================================================================
// Supporting Enums
// ============================================================================

/// Strategy used to match query terms within documents.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MatchingStrategy {
    /// Remove query words from last to first (default).
    #[default]
    Last,
    /// All query words are mandatory.
    All,
    /// Remove query words from most frequent to least frequent.
    Frequency,
}

/// Hybrid search configuration combining keyword and semantic search.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridQuery {
    /// Balance between keyword (0.0) and semantic (1.0) search. Default: 0.5.
    #[serde(default = "default_semantic_ratio")]
    pub semantic_ratio: f32,
    /// Name of the embedder to use.
    pub embedder: String,
}

fn default_semantic_ratio() -> f32 { 0.5 }

impl HybridQuery {
    /// Create a hybrid query for the named embedder with a default 0.5 semantic ratio.
    pub fn new(embedder: impl Into<String>) -> Self {
        Self {
            semantic_ratio: 0.5,
            embedder: embedder.into(),
        }
    }

    /// Set the semantic ratio (clamped to 0.0..=1.0). 0.0 = keyword only, 1.0 = semantic only.
    pub fn with_semantic_ratio(mut self, ratio: f32) -> Self {
        self.semantic_ratio = ratio.clamp(0.0, 1.0);
        self
    }
}

// ============================================================================
// SearchHit
// ============================================================================

/// A single search hit with document and optional metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    /// The document data (flattened into the hit object).
    #[serde(flatten)]
    pub document: Value,

    /// Highlighted/cropped version of the document.
    #[serde(default, rename = "_formatted", skip_serializing_if = "Option::is_none")]
    pub formatted: Option<Value>,

    /// Positions of matching terms in the document.
    #[serde(default, rename = "_matchesPosition", skip_serializing_if = "Option::is_none")]
    pub matches_position: Option<BTreeMap<String, Vec<MatchBounds>>>,

    /// Global ranking score (0.0-1.0).
    #[serde(default, rename = "_rankingScore", skip_serializing_if = "Option::is_none")]
    pub ranking_score: Option<f64>,

    /// Detailed ranking score breakdown per ranking rule.
    #[serde(default, rename = "_rankingScoreDetails", skip_serializing_if = "Option::is_none")]
    pub ranking_score_details: Option<Map<String, Value>>,

    /// Vector similarity score (vector/hybrid search only).
    #[serde(default, rename = "_semanticScore", skip_serializing_if = "Option::is_none")]
    pub semantic_score: Option<f32>,

    /// Vectors associated with the document.
    #[serde(default, rename = "_vectors", skip_serializing_if = "Option::is_none")]
    pub vectors: Option<Value>,
}

impl SearchHit {
    /// Create a new search hit with a document and optional ranking score.
    pub fn new(document: Value, ranking_score: Option<f64>) -> Self {
        Self {
            document,
            formatted: None,
            matches_position: None,
            ranking_score,
            ranking_score_details: None,
            semantic_score: None,
            vectors: None,
        }
    }
}

/// Bounds of a match within a document field value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchBounds {
    /// Byte offset where the match begins.
    pub start: usize,
    /// Length of the match in bytes.
    pub length: usize,
}

// ============================================================================
// SearchResult
// ============================================================================

/// Pagination metadata. Serializes to one of two shapes via `#[serde(untagged)]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum HitsInfo {
    /// Page-based pagination with exhaustive counts.
    #[serde(rename_all = "camelCase")]
    Pagination {
        /// Number of hits requested per page.
        hits_per_page: usize,
        /// Current page number (1-indexed).
        page: usize,
        /// Total number of pages available.
        total_pages: usize,
        /// Exact total number of matching documents.
        total_hits: usize,
    },
    /// Offset/limit pagination with estimated counts.
    #[serde(rename_all = "camelCase")]
    OffsetLimit {
        /// Maximum number of hits requested.
        limit: usize,
        /// Number of hits skipped.
        offset: usize,
        /// Estimated total number of matching documents.
        estimated_total_hits: usize,
    },
}

/// Numeric facet statistics (min/max) for a single faceted attribute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FacetStats {
    /// Minimum numeric value found for the facet.
    pub min: f64,
    /// Maximum numeric value found for the facet.
    pub max: f64,
}

/// The result of a search operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    /// The matching documents.
    pub hits: Vec<SearchHit>,

    /// Internal milli document IDs corresponding 1:1 to `hits`.
    ///
    /// Used by hybrid search to merge keyword and vector results by actual
    /// document identity rather than hit index. Not serialized to JSON.
    #[serde(skip)]
    pub document_ids: Vec<u32>,

    /// The original query string.
    pub query: String,

    /// Time taken to process the search in milliseconds.
    pub processing_time_ms: u128,

    /// Pagination info (flattened -- one variant active per response).
    #[serde(flatten)]
    pub hits_info: HitsInfo,

    /// Facet value distribution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facet_distribution: Option<BTreeMap<String, IndexMap<String, u64>>>,

    /// Numeric facet statistics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facet_stats: Option<BTreeMap<String, FacetStats>>,

    /// Number of hits from semantic/vector search (hybrid only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_hit_count: Option<u32>,
}

impl SearchResult {
    /// Create a new search result with offset/limit pagination.
    pub fn new(
        hits: Vec<SearchHit>,
        query: String,
        processing_time_ms: u128,
        estimated_total_hits: usize,
        limit: usize,
        offset: usize,
    ) -> Self {
        Self {
            hits,
            document_ids: Vec::new(),
            query,
            processing_time_ms,
            hits_info: HitsInfo::OffsetLimit {
                limit,
                offset,
                estimated_total_hits,
            },
            facet_distribution: None,
            facet_stats: None,
            semantic_hit_count: None,
        }
    }

    /// Create a new search result with offset/limit pagination and milli document IDs.
    pub fn with_document_ids(
        hits: Vec<SearchHit>,
        document_ids: Vec<u32>,
        query: String,
        processing_time_ms: u128,
        estimated_total_hits: usize,
        limit: usize,
        offset: usize,
    ) -> Self {
        Self {
            hits,
            document_ids,
            query,
            processing_time_ms,
            hits_info: HitsInfo::OffsetLimit {
                limit,
                offset,
                estimated_total_hits,
            },
            facet_distribution: None,
            facet_stats: None,
            semantic_hit_count: None,
        }
    }

    /// Create a new search result with page-based pagination.
    pub fn new_paginated(
        hits: Vec<SearchHit>,
        query: String,
        processing_time_ms: u128,
        total_hits: usize,
        page: usize,
        hits_per_page: usize,
    ) -> Self {
        let total_pages = if hits_per_page > 0 {
            (total_hits + hits_per_page - 1) / hits_per_page
        } else {
            0
        };
        Self {
            hits,
            document_ids: Vec::new(),
            query,
            processing_time_ms,
            hits_info: HitsInfo::Pagination {
                hits_per_page,
                page,
                total_pages,
                total_hits,
            },
            facet_distribution: None,
            facet_stats: None,
            semantic_hit_count: None,
        }
    }
}

// ============================================================================
// Legacy Hybrid Search Types (backward compat, delegates to SearchQuery)
// ============================================================================

/// Query parameters for hybrid search requests.
///
/// DEPRECATED: Use SearchQuery with `hybrid` and `vector` fields instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridSearchQuery {
    /// The base search query.
    #[serde(flatten)]
    pub search: SearchQuery,

    /// The query vector for semantic search.
    #[serde(default)]
    pub vector: Option<Vec<f32>>,

    /// The ratio of semantic vs keyword search.
    #[serde(default = "default_semantic_ratio")]
    pub semantic_ratio: f32,
}

impl HybridSearchQuery {
    /// Create a new hybrid search query with the given search string.
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            search: SearchQuery::new(query),
            vector: None,
            semantic_ratio: 0.5,
        }
    }

    /// Set the query vector for semantic search.
    pub fn with_vector(mut self, vector: Vec<f32>) -> Self { self.vector = Some(vector); self }
    /// Set the semantic ratio (clamped to 0.0..=1.0).
    pub fn with_semantic_ratio(mut self, ratio: f32) -> Self { self.semantic_ratio = ratio.clamp(0.0, 1.0); self }
    /// Set the maximum number of results to return.
    pub fn with_limit(mut self, limit: usize) -> Self { self.search = self.search.with_limit(limit); self }
    /// Set the number of results to skip.
    pub fn with_offset(mut self, offset: usize) -> Self { self.search = self.search.with_offset(offset); self }
    /// Set a filter expression.
    pub fn with_filter(mut self, filter: impl Into<Value>) -> Self { self.search = self.search.with_filter(filter); self }
    /// Include `_rankingScore` in each hit.
    pub fn with_ranking_score(mut self, show: bool) -> Self { self.search = self.search.with_ranking_score(show); self }
}

/// Result of a hybrid search operation.
///
/// DEPRECATED: Use SearchResult with `semantic_hit_count` field instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridSearchResult {
    /// The underlying search result (hits, pagination, facets).
    #[serde(flatten)]
    pub result: SearchResult,
    /// Number of hits that came from semantic/vector search.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_hit_count: Option<u32>,
}

impl HybridSearchResult {
    /// Create a new hybrid search result from a search result and semantic hit count.
    pub fn new(result: SearchResult, semantic_hit_count: Option<u32>) -> Self {
        Self { result, semantic_hit_count }
    }
}

// ============================================================================
// Multi-Search Types
// ============================================================================

/// A single query in a multi-search request, targeting a specific index.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiSearchQuery {
    /// The UID of the index to search.
    pub index_uid: String,
    /// The search query parameters.
    #[serde(flatten)]
    pub query: SearchQuery,
}

/// Result of a non-federated multi-search.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiSearchResult {
    /// Per-index search results in the same order as the input queries.
    pub results: Vec<SearchResultWithIndex>,
}

/// A search result tagged with the index it came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResultWithIndex {
    /// The UID of the index this result came from.
    pub index_uid: String,
    /// The search result for this index.
    #[serde(flatten)]
    pub result: SearchResult,
}

// ============================================================================
// Federated Multi-Search Types
// ============================================================================

/// Configuration for federated (merged) multi-search.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Federation {
    /// Maximum number of merged hits to return. Default: 20.
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Number of merged hits to skip. Default: 0.
    #[serde(default)]
    pub offset: usize,
    /// Request page-based pagination instead of offset/limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
    /// Number of hits per page (page-based pagination).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hits_per_page: Option<usize>,
    /// Per-index facet attributes to compute distribution for.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub facets_by_index: BTreeMap<String, Option<Vec<String>>>,
    /// Configuration for merging facets across indexes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_facets: Option<MergeFacets>,
}

/// Configuration for merging facet distributions across indexes in a federated search.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeFacets {
    /// Maximum number of facet values to return per attribute after merging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_values_per_facet: Option<usize>,
}

/// Per-query options that influence ranking in a federated multi-search.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FederationOptions {
    /// Weight multiplier applied to ranking scores from this query. Default: 1.0.
    #[serde(default = "default_federation_weight")]
    pub weight: f64,
    /// Position of this query for tie-breaking. Lower values rank higher.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_position: Option<usize>,
}

fn default_federation_weight() -> f64 { 1.0 }

/// A single query in a federated multi-search request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FederatedMultiSearchQuery {
    /// The UID of the index to search.
    pub index_uid: String,
    /// The search query parameters.
    #[serde(flatten)]
    pub query: SearchQuery,
    /// Optional weight and position for cross-index ranking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub federation_options: Option<FederationOptions>,
}

/// Result of a federated multi-search.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FederatedSearchResult {
    /// Merged hits from all queried indexes, sorted by ranking score.
    pub hits: Vec<SearchHit>,
    /// Total processing time in milliseconds.
    pub processing_time_ms: u128,
    /// Pagination metadata for the merged result set.
    #[serde(flatten)]
    pub hits_info: HitsInfo,
    /// Merged facet value distribution (when `merge_facets` is set).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facet_distribution: Option<BTreeMap<String, IndexMap<String, u64>>>,
    /// Merged numeric facet statistics (when `merge_facets` is set).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facet_stats: Option<BTreeMap<String, FacetStats>>,
    /// Per-index facet distributions and statistics.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub facets_by_index: BTreeMap<String, ComputedFacets>,
    /// Number of hits from semantic/vector search across all queries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_hit_count: Option<u32>,
}

/// Facet distribution and statistics computed for a single index in a federated search.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputedFacets {
    /// Facet value distribution (attribute name -> value -> count).
    pub distribution: BTreeMap<String, IndexMap<String, u64>>,
    /// Numeric facet statistics (attribute name -> min/max).
    pub stats: BTreeMap<String, FacetStats>,
}

// ============================================================================
// Facet Search Types
// ============================================================================

/// Query for searching within facet values of an index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FacetSearchQuery {
    /// The facet attribute to search within.
    pub facet_name: String,
    /// Query string to filter facet values (autocomplete-style).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facet_query: Option<String>,
    /// Optional keyword query to scope the facet search to matching documents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
    /// Optional filter expression to further restrict candidate documents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Value>,
    /// Strategy for matching query terms.
    #[serde(default)]
    pub matching_strategy: MatchingStrategy,
    /// Restrict the keyword search to specific attributes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes_to_search_on: Option<Vec<String>>,
    /// Minimum ranking score threshold for the keyword search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranking_score_threshold: Option<f64>,
    /// Locales for language-specific tokenization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locales: Option<Vec<String>>,
}

/// Result of a facet search.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FacetSearchResult {
    /// Matching facet values with their document counts.
    pub facet_hits: Vec<FacetHit>,
    /// The facet query that was used (echoed back).
    pub facet_query: Option<String>,
    /// Time taken to process the facet search in milliseconds.
    pub processing_time_ms: u128,
}

/// A single facet value with its document count.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FacetHit {
    /// The facet value string.
    pub value: String,
    /// Number of documents with this facet value.
    pub count: u64,
}

// ============================================================================
// Similar Documents Types
// ============================================================================

/// Query to find documents similar to a reference document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarQuery {
    /// The document ID to find similar documents for.
    pub id: Value,
    /// Number of results to skip.
    #[serde(default)]
    pub offset: usize,
    /// Maximum number of similar documents to return. Default: 20.
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Filter expression to restrict candidate documents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Value>,
    /// Name of the embedder to use for similarity computation.
    pub embedder: String,
    /// Attributes to include in returned documents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes_to_retrieve: Option<BTreeSet<String>>,
    /// Whether to include embedding vectors in returned documents.
    #[serde(default)]
    pub retrieve_vectors: bool,
    /// Include `_rankingScore` in each hit.
    #[serde(default)]
    pub show_ranking_score: bool,
    /// Include `_rankingScoreDetails` in each hit.
    #[serde(default)]
    pub show_ranking_score_details: bool,
    /// Minimum ranking score threshold; hits below this are excluded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranking_score_threshold: Option<f64>,
}

/// Result of a similar documents query.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarResult {
    /// Documents similar to the reference document.
    pub hits: Vec<SearchHit>,
    /// The ID of the reference document (echoed back).
    pub id: String,
    /// Time taken to process the query in milliseconds.
    pub processing_time_ms: u128,
    /// Pagination metadata.
    #[serde(flatten)]
    pub hits_info: HitsInfo,
}

// ============================================================================
// GetDocumentsOptions
// ============================================================================

/// Options for fetching documents with filtering, field selection, and pagination.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetDocumentsOptions {
    /// Number of documents to skip. Default: 0.
    #[serde(default)]
    pub offset: usize,
    /// Maximum number of documents to return. Default: 20.
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Specific fields to include in the response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
    /// Whether to include embedding vectors in returned documents.
    #[serde(default)]
    pub retrieve_vectors: bool,
    /// Filter expression to select matching documents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Value>,
    /// Fetch specific documents by their IDs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<String>>,
    /// Sort expressions, e.g. `["price:asc"]`. Only applies to filtered results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<Vec<String>>,
}
