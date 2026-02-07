# Meilisearch Embedded Library -- SDK Coverage Design

## 1. Overview

This document specifies the technical design for achieving comprehensive Meilisearch SDK coverage in `meilisearch-lib`. The library provides an embedded, in-process interface to Meilisearch, bypassing the HTTP layer to interact directly with the `milli` indexing engine via LMDB-backed storage.

### 1.1 Scope

**In scope** (operations that make sense for an embedded library):

- Index operations (create, get, list, delete, swap, update primary key)
- Document operations (add, update/partial, get, delete, clear)
- Search operations (keyword, hybrid, multi-search, facet search, similar documents)
- Settings operations (bulk get/update/reset, individual per-setting accessors)
- Instance info (health, version, stats)
- Backup operations (dump, snapshot)
- Experimental features configuration

**Out of scope** (server-only concerns):

- Network/federation across remote instances
- Chat completions (LLM orchestration)
- Webhooks, logs route, metrics route
- Export route
- API keys / authentication
- Task queuing and batches (embedded operations are synchronous)
- HTTP-level concerns (rate limiting, CORS, proxying)

### 1.2 Guiding Principles

1. **Embedded-first**: Operations execute synchronously. No task queue indirection.
2. **API shape parity**: Types serialize to the same JSON shape as the HTTP API (using `serde(rename_all = "camelCase")`).
3. **Zero-copy where possible**: Reuse `milli` types directly rather than converting through intermediate representations.
4. **Builder pattern**: Query and settings types use builder methods for ergonomic construction.
5. **Errors as values**: All fallible operations return `Result<T, Error>`. No panics on user input.

---

## 2. Architecture

### 2.1 Current Architecture

```
+---------------------------------------------------------------+
|                        meilisearch-lib                        |
+---------------------------------------------------------------+
|  Meilisearch                     Index                        |
|  +-- create_index()              +-- add_documents()          |
|  +-- get_index()                 +-- get_document()           |
|  +-- list_indexes()              +-- get_documents()          |
|  +-- delete_index()              +-- delete_document()        |
|  +-- index_exists()              +-- delete_documents()       |
|  +-- index_stats()               +-- delete_by_filter()       |
|  +-- with_vector_store()         +-- clear()                  |
|                                  +-- document_count()         |
|  SearchQuery (6 fields)          +-- search()                 |
|  SearchResult (6 fields)         +-- search_simple()          |
|  SearchHit (2 fields)            +-- search_vectors()         |
|  HybridSearchQuery               +-- hybrid_search()          |
|  HybridSearchResult              +-- get_settings()           |
|  Settings (15 fields)            +-- update_settings()        |
|  EmbedderSettings                +-- reset_settings()         |
|                                  +-- primary_key()            |
+---------------------------------------------------------------+
|  Preprocessing         RAG               Vector               |
|  +-- QueryPipeline     +-- RagPipeline   +-- VectorStore      |
|  +-- TypoCorrector     +-- Retriever     +-- NoOpVectorStore  |
|  +-- SynonymMap        +-- Reranker      +-- SurrealDbVector  |
|                        +-- Generator                          |
+---------------------------------------------------------------+
|                          milli                                |
|  (Core indexing engine -- LMDB-backed via heed)               |
+---------------------------------------------------------------+
```

**Current limitations:**

- `SearchQuery` has only 6 fields (query, limit, offset, filter, attributes_to_retrieve, show_ranking_score). The HTTP API `SearchQuery` has 28+ fields.
- `SearchResult` lacks facet distribution, facet stats, pagination modes, and semantic hit count.
- `SearchHit` lacks formatted fields, matches position, and ranking score details.
- No multi-search, facet search, or similar documents support.
- `Settings` has 15 flat fields. The HTTP API groups typo tolerance, faceting, and pagination into structured sub-objects and has 22 fields.
- No individual per-setting accessors (get/update/reset for each setting).
- No index swap, dump, snapshot, experimental features, or instance info.
- No document update (partial merge) or advanced document fetching (filter, fields, sort).

### 2.2 Target Architecture

```
+---------------------------------------------------------------+
|                        meilisearch-lib                        |
+---------------------------------------------------------------+
|  Meilisearch (facade)                                         |
|  +-- Index Operations                                         |
|  |   create, get, list, delete, swap, update_primary_key      |
|  +-- Multi-Search          (multi_search, multi_search_fed)   |
|  +-- Instance Info         (health, version, stats)           |
|  +-- Backup                (create_dump, create_snapshot)     |
|  +-- Experimental Features (get, update)                      |
+---------------------------------------------------------------+
|  Index (per-index handle)                                     |
|  +-- Documents                                                |
|  |   add, update(partial), get_one, get_many, get_with_opts,  |
|  |   delete_one, delete_many, delete_by_filter, clear         |
|  +-- Search                                                   |
|  |   search (full params), search_simple, hybrid_search,      |
|  |   facet_search, get_similar_documents                      |
|  +-- Settings                                                 |
|  |   get/update/reset (bulk), 20x individual per-setting      |
|  |   accessors (get_*/update_*/reset_*)                       |
|  +-- Stats                                                    |
|      primary_key, document_count, stats                       |
+---------------------------------------------------------------+
|  Types (API-compatible, serde camelCase)                      |
|  +-- SearchQuery (28 fields)    SearchResult (full)           |
|  +-- SearchHit (formatted, matches, score details)            |
|  +-- MultiSearchQuery           FederatedSearchResult         |
|  +-- FacetSearchQuery/Result    SimilarQuery/Result           |
|  +-- Settings (structured)      TypoToleranceSettings         |
|  +-- FacetingSettings           PaginationSettings            |
|  +-- GetDocumentsOptions        DocumentsResult               |
|  +-- HealthStatus, VersionInfo, GlobalStats                   |
|  +-- ExperimentalFeatures       DumpInfo                      |
|  +-- Error (extended enum)                                    |
+---------------------------------------------------------------+
|  Preprocessing         RAG               Vector               |
|  (unchanged)           (unchanged)       (unchanged)          |
+---------------------------------------------------------------+
|                          milli                                |
+---------------------------------------------------------------+
```

---

## 3. Component Designs

### 3.1 Enhanced Search Types

#### 3.1.1 SearchQuery

Extended from the current 6 fields to match the HTTP API's full parameter set. Fields irrelevant to embedded mode (`use_network`, `media`, `personalize`) are omitted.

```rust
use std::collections::{BTreeSet, HashSet};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Query parameters for search requests.
///
/// Matches the Meilisearch HTTP API search body (minus server-only fields).
/// All fields use camelCase serialization for API shape parity.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    // -- Core query --

    /// The search query string. If `None`, matches all documents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,

    /// Pre-computed query vector for pure vector search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector: Option<Vec<f32>>,

    /// Hybrid search configuration (combines keyword + semantic).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hybrid: Option<HybridQuery>,

    // -- Pagination (offset/limit mode) --

    /// Number of results to skip. Default: 0.
    #[serde(default)]
    pub offset: usize,

    /// Maximum number of results to return. Default: 20.
    #[serde(default = "default_limit")]
    pub limit: usize,

    /// Request a specific page (switches to page-based pagination).
    /// Mutually exclusive with offset/limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,

    /// Hits per page (switches to page-based pagination).
    /// Mutually exclusive with offset/limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hits_per_page: Option<usize>,

    // -- Attribute selection --

    /// Attributes to include in returned documents. `None` = all displayed attributes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes_to_retrieve: Option<BTreeSet<String>>,

    /// Whether to include embedding vectors in returned documents.
    #[serde(default)]
    pub retrieve_vectors: bool,

    // -- Highlighting and cropping --

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

    // -- Scoring and ranking --

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

    // -- Filtering and sorting --

    /// Filter expression. Supports string or array-of-arrays syntax.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Value>,

    /// Sort expressions, e.g. `["price:asc", "rating:desc"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<Vec<String>>,

    /// Return only documents with distinct values for this attribute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distinct: Option<String>,

    // -- Facets --

    /// Facet attributes to compute distribution for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facets: Option<Vec<String>>,

    // -- Search behavior --

    /// Strategy for matching query terms: `Last` (default), `All`, `Frequency`.
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
```

Builder methods follow the existing pattern:

```rust
impl SearchQuery {
    pub fn new(query: impl Into<String>) -> Self { /* ... */ }
    pub fn match_all() -> Self { Self::default() }
    pub fn with_limit(mut self, limit: usize) -> Self { /* ... */ }
    pub fn with_offset(mut self, offset: usize) -> Self { /* ... */ }
    pub fn with_page(mut self, page: usize) -> Self { /* ... */ }
    pub fn with_hits_per_page(mut self, hpp: usize) -> Self { /* ... */ }
    pub fn with_filter(mut self, filter: impl Into<Value>) -> Self { /* ... */ }
    pub fn with_sort(mut self, sort: Vec<String>) -> Self { /* ... */ }
    pub fn with_facets(mut self, facets: Vec<String>) -> Self { /* ... */ }
    pub fn with_attributes_to_retrieve(mut self, attrs: BTreeSet<String>) -> Self { /* ... */ }
    pub fn with_attributes_to_highlight(mut self, attrs: HashSet<String>) -> Self { /* ... */ }
    pub fn with_attributes_to_crop(mut self, attrs: Vec<String>) -> Self { /* ... */ }
    pub fn with_ranking_score(mut self, show: bool) -> Self { /* ... */ }
    pub fn with_ranking_score_details(mut self, show: bool) -> Self { /* ... */ }
    pub fn with_matches_position(mut self, show: bool) -> Self { /* ... */ }
    pub fn with_matching_strategy(mut self, strategy: MatchingStrategy) -> Self { /* ... */ }
    pub fn with_ranking_score_threshold(mut self, threshold: f64) -> Self { /* ... */ }
    pub fn with_distinct(mut self, attr: impl Into<String>) -> Self { /* ... */ }
    pub fn with_locales(mut self, locales: Vec<String>) -> Self { /* ... */ }
    pub fn with_attributes_to_search_on(mut self, attrs: Vec<String>) -> Self { /* ... */ }
}
```

#### 3.1.2 HybridQuery

```rust
/// Hybrid search configuration combining keyword and semantic search.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridQuery {
    /// Balance between keyword (0.0) and semantic (1.0) search. Default: 0.5.
    #[serde(default = "default_semantic_ratio")]
    pub semantic_ratio: f32,

    /// Name of the embedder to use (must match an embedder configured in settings).
    pub embedder: String,
}

fn default_semantic_ratio() -> f32 { 0.5 }
```

#### 3.1.3 MatchingStrategy

```rust
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

impl From<MatchingStrategy> for milli::TermsMatchingStrategy {
    fn from(s: MatchingStrategy) -> Self {
        match s {
            MatchingStrategy::Last => Self::Last,
            MatchingStrategy::All => Self::All,
            MatchingStrategy::Frequency => Self::Frequency,
        }
    }
}
```

#### 3.1.4 SearchResult

```rust
use std::collections::BTreeMap;
use indexmap::IndexMap;

/// The result of a search operation.
///
/// Supports two pagination modes:
/// - Offset/limit (default): `estimated_total_hits`, `limit`, `offset`
/// - Page-based: `total_hits`, `total_pages`, `page`, `hits_per_page`
///
/// Only one set of pagination fields will be present, determined by
/// whether the query used `page`/`hits_per_page` or `offset`/`limit`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    /// The matching documents.
    pub hits: Vec<SearchHit>,

    /// The original query string.
    pub query: String,

    /// The query vector, if vector search was used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_vector: Option<Vec<f32>>,

    /// Time taken to process the search in milliseconds.
    pub processing_time_ms: u128,

    /// Pagination info (flattened -- one variant active per response).
    #[serde(flatten)]
    pub hits_info: HitsInfo,

    /// Facet value distribution, keyed by facet attribute name.
    /// Present only when `facets` was set in the query.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facet_distribution: Option<BTreeMap<String, IndexMap<String, u64>>>,

    /// Numeric facet statistics (min/max), keyed by facet attribute name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facet_stats: Option<BTreeMap<String, FacetStats>>,

    /// Number of hits originating from semantic/vector search (hybrid only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_hit_count: Option<u32>,
}

/// Pagination metadata. Serializes to one of two shapes via `#[serde(untagged)]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum HitsInfo {
    /// Page-based pagination with exhaustive counts.
    #[serde(rename_all = "camelCase")]
    Pagination {
        hits_per_page: usize,
        page: usize,
        total_pages: usize,
        total_hits: usize,
    },
    /// Offset/limit pagination with estimated counts.
    #[serde(rename_all = "camelCase")]
    OffsetLimit {
        limit: usize,
        offset: usize,
        estimated_total_hits: usize,
    },
}

/// Numeric facet statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FacetStats {
    pub min: f64,
    pub max: f64,
}
```

#### 3.1.5 SearchHit

```rust
use std::collections::BTreeMap;
use serde_json::{Map, Value};

/// A single search hit with document and optional metadata.
///
/// Metadata fields (`_formatted`, `_matchesPosition`, `_rankingScore`,
/// `_rankingScoreDetails`) use underscore-prefixed keys matching the HTTP API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    /// The document data (flattened into the hit object).
    #[serde(flatten)]
    pub document: Value,

    /// Highlighted/cropped version of the document.
    /// Present when `attributes_to_highlight` or `attributes_to_crop` are set.
    #[serde(default, rename = "_formatted", skip_serializing_if = "Option::is_none")]
    pub formatted: Option<Value>,

    /// Positions of matching terms in the document, keyed by attribute name.
    /// Present when `show_matches_position` is `true`.
    #[serde(default, rename = "_matchesPosition", skip_serializing_if = "Option::is_none")]
    pub matches_position: Option<BTreeMap<String, Vec<MatchBounds>>>,

    /// Global ranking score (0.0-1.0).
    /// Present when `show_ranking_score` is `true`.
    #[serde(default, rename = "_rankingScore", skip_serializing_if = "Option::is_none")]
    pub ranking_score: Option<f64>,

    /// Detailed ranking score breakdown per ranking rule.
    /// Present when `show_ranking_score_details` is `true`.
    #[serde(default, rename = "_rankingScoreDetails", skip_serializing_if = "Option::is_none")]
    pub ranking_score_details: Option<Map<String, Value>>,

    /// Vector similarity score (vector/hybrid search only).
    #[serde(default, rename = "_semanticScore", skip_serializing_if = "Option::is_none")]
    pub semantic_score: Option<f32>,

    /// Vectors associated with the document (when `retrieve_vectors` is `true`).
    #[serde(default, rename = "_vectors", skip_serializing_if = "Option::is_none")]
    pub vectors: Option<Value>,
}

/// Bounds of a match within a document field value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchBounds {
    pub start: usize,
    pub length: usize,
}
```

**Implementation note:** The `_formatted`, `_matchesPosition`, and `_rankingScoreDetails` fields require calling into milli's `MatcherBuilder`, `FormatOptions`, and `ScoreDetails` APIs. The `ScoreDetails` from milli are serialized as a `serde_json::Map` to preserve the ranking rule names without needing to enumerate every possible rule type.

---

### 3.2 Multi-Search Types

#### 3.2.1 Non-Federated Multi-Search

```rust
/// A single query in a multi-search request, targeting a specific index.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiSearchQuery {
    /// Target index UID.
    pub index_uid: String,

    /// The search query (all SearchQuery fields).
    #[serde(flatten)]
    pub query: SearchQuery,
}

/// Result of a non-federated multi-search: one SearchResult per query.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiSearchResult {
    pub results: Vec<SearchResultWithIndex>,
}

/// A search result tagged with the index it came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResultWithIndex {
    pub index_uid: String,
    #[serde(flatten)]
    pub result: SearchResult,
}

impl Meilisearch {
    /// Execute multiple searches across indexes. Returns one result per query.
    pub fn multi_search(&self, queries: Vec<MultiSearchQuery>) -> Result<MultiSearchResult>;
}
```

#### 3.2.2 Federated Multi-Search

```rust
/// Configuration for federated (merged) multi-search.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Federation {
    /// Maximum number of merged results. Default: 20.
    #[serde(default = "default_limit")]
    pub limit: usize,

    /// Number of results to skip. Default: 0.
    #[serde(default)]
    pub offset: usize,

    /// Page-based pagination (alternative to offset/limit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,

    /// Hits per page (alternative to offset/limit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hits_per_page: Option<usize>,

    /// Facets to compute per index. Key = index UID, value = facet attributes
    /// (None = use query's facets).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub facets_by_index: BTreeMap<String, Option<Vec<String>>>,

    /// Options for merging facets across indexes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_facets: Option<MergeFacets>,
}

/// Options controlling how facets are merged across indexes.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeFacets {
    /// Maximum facet values per facet after merging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_values_per_facet: Option<usize>,
}

/// Per-query federation options (weight, position).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FederationOptions {
    /// Weight applied to this query's scores in the merged ranking.
    /// Default: 1.0. Must be >= 0.0.
    #[serde(default = "default_federation_weight")]
    pub weight: f64,

    /// Override the query's position for the `queriesPosition` field in hits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_position: Option<usize>,
}

fn default_federation_weight() -> f64 { 1.0 }

/// A query in a federated multi-search, with optional federation options.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FederatedMultiSearchQuery {
    pub index_uid: String,
    #[serde(flatten)]
    pub query: SearchQuery,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub federation_options: Option<FederationOptions>,
}

/// Result of a federated multi-search (merged hits with provenance).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FederatedSearchResult {
    /// Merged hits from all queries, ranked by weighted score.
    pub hits: Vec<SearchHit>,

    /// Processing time in milliseconds.
    pub processing_time_ms: u128,

    /// Pagination info.
    #[serde(flatten)]
    pub hits_info: HitsInfo,

    /// Merged facet distribution (when `merge_facets` is set).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facet_distribution: Option<BTreeMap<String, IndexMap<String, u64>>>,

    /// Merged facet stats.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facet_stats: Option<BTreeMap<String, FacetStats>>,

    /// Facet distributions grouped by index UID.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub facets_by_index: BTreeMap<String, ComputedFacets>,

    /// Semantic hit count (hybrid only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_hit_count: Option<u32>,
}

/// Facet data for a single index in a federated result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputedFacets {
    pub distribution: BTreeMap<String, IndexMap<String, u64>>,
    pub stats: BTreeMap<String, FacetStats>,
}

impl Meilisearch {
    /// Execute federated multi-search with merged results.
    pub fn multi_search_federated(
        &self,
        queries: Vec<FederatedMultiSearchQuery>,
        federation: Federation,
    ) -> Result<FederatedSearchResult>;
}
```

**Implementation note:** Each hit in a `FederatedSearchResult` carries `_federation` metadata (index UID, query position, weighted ranking score) embedded in the document's `Value` via serde flattening, matching the HTTP API's behavior.

---

### 3.3 Facet Search Types

```rust
/// Query for searching within facet values of an index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FacetSearchQuery {
    /// The facet attribute to search within (required).
    pub facet_name: String,

    /// Query string to match against facet values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facet_query: Option<String>,

    /// Text query to filter the document set before computing facets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,

    /// Filter expression to narrow the document set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Value>,

    /// Matching strategy for the text query.
    #[serde(default)]
    pub matching_strategy: MatchingStrategy,

    /// Restrict search to these attributes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes_to_search_on: Option<Vec<String>>,

    /// Minimum ranking score threshold for documents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranking_score_threshold: Option<f64>,

    /// Language locales for query processing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locales: Option<Vec<String>>,

    /// When true, returns exhaustive facet counts instead of estimates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exhaustive_facet_count: Option<bool>,

    /// Hybrid search configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hybrid: Option<HybridQuery>,

    /// Pre-computed query vector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector: Option<Vec<f32>>,
}

/// Result of a facet search.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FacetSearchResult {
    /// Matching facet values with counts.
    pub facet_hits: Vec<FacetHit>,

    /// The facet query that was used.
    pub facet_query: Option<String>,

    /// Processing time in milliseconds.
    pub processing_time_ms: u128,
}

/// A single facet value with its document count.
///
/// This reuses milli's `FacetValueHit` shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FacetHit {
    /// The facet value string.
    pub value: String,

    /// Number of documents matching this facet value.
    pub count: u64,
}

impl Index {
    /// Search within facet values of an attribute.
    ///
    /// The attribute must be configured as filterable in the index settings.
    pub fn facet_search(&self, query: &FacetSearchQuery) -> Result<FacetSearchResult>;
}
```

**Implementation:** Delegates to `milli::SearchForFacetValues` which is already available in the engine.

---

### 3.4 Similar Documents Types

```rust
/// Query to find documents similar to a reference document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarQuery {
    /// The document ID to find similar documents for.
    pub id: Value,

    /// Number of results to skip. Default: 0.
    #[serde(default)]
    pub offset: usize,

    /// Maximum number of results. Default: 20.
    #[serde(default = "default_limit")]
    pub limit: usize,

    /// Filter expression to apply to candidates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Value>,

    /// Name of the embedder to use for similarity computation (required).
    pub embedder: String,

    /// Attributes to include in returned documents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes_to_retrieve: Option<BTreeSet<String>>,

    /// Whether to include vector data in results.
    #[serde(default)]
    pub retrieve_vectors: bool,

    /// Include ranking score in results.
    #[serde(default)]
    pub show_ranking_score: bool,

    /// Include ranking score details.
    #[serde(default)]
    pub show_ranking_score_details: bool,

    /// Minimum ranking score threshold (0.0-1.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranking_score_threshold: Option<f64>,
}

/// Result of a similar documents query.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarResult {
    /// Similar documents.
    pub hits: Vec<SearchHit>,

    /// The reference document ID.
    pub id: String,

    /// Processing time in milliseconds.
    pub processing_time_ms: u128,

    /// Pagination information.
    #[serde(flatten)]
    pub hits_info: HitsInfo,
}

impl Index {
    /// Find documents similar to a given document using vector embeddings.
    ///
    /// Requires an embedder to be configured in the index settings.
    pub fn get_similar_documents(&self, query: &SimilarQuery) -> Result<SimilarResult>;
}
```

**Implementation:** Retrieves the reference document's embedding, then performs a vector search via `milli::Search` in `SemanticOnly` mode with the retrieved embedding as the query vector.

---

### 3.5 Enhanced Document Operations

#### 3.5.1 Update Documents (Partial Merge)

```rust
impl Index {
    /// Partially update documents by merging fields.
    ///
    /// Unlike `add_documents()` which replaces entire documents,
    /// this merges the provided fields with existing document fields.
    /// Uses milli's `IndexDocuments` with `UpdateDocuments` method.
    pub fn update_documents(
        &self,
        documents: Vec<Value>,
        primary_key: Option<&str>,
    ) -> Result<u64>;
}
```

**Implementation:** Uses `milli::update::IndexDocumentsConfig` with `update_method: IndexDocumentsMethod::UpdateDocuments` (the current `add_documents` uses `ReplaceDocuments`).

#### 3.5.2 Advanced Document Retrieval

```rust
/// Options for fetching documents with filtering, field selection, sorting, and pagination.
///
/// Matches the POST `/indexes/{uid}/documents/fetch` endpoint body.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetDocumentsOptions {
    /// Number of documents to skip. Default: 0.
    #[serde(default)]
    pub offset: usize,

    /// Maximum number of documents to return. Default: 20.
    #[serde(default = "default_limit")]
    pub limit: usize,

    /// Attributes to include in returned documents. `None` = all displayed attributes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,

    /// Whether to include vector data.
    #[serde(default)]
    pub retrieve_vectors: bool,

    /// Specific document IDs to fetch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<Value>>,

    /// Filter expression to narrow results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Value>,

    /// Sort expressions, e.g. `["price:asc"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<Vec<String>>,
}

/// Result of a paginated document retrieval operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentsResult {
    /// The retrieved documents.
    pub results: Vec<Value>,
    /// Total number of documents matching the criteria.
    pub total: u64,
    /// Offset used.
    pub offset: usize,
    /// Limit used.
    pub limit: usize,
}

impl Index {
    /// Get a single document by external ID with optional field selection.
    pub fn get_document(
        &self,
        id: &str,
        fields: Option<&[String]>,
    ) -> Result<Option<Value>>;

    /// Get documents with advanced options (filtering, sorting, pagination).
    pub fn get_documents(&self, options: &GetDocumentsOptions) -> Result<DocumentsResult>;
}
```

---

### 3.6 Enhanced Settings Types

The current flat `Settings` struct (15 fields) is restructured to match the HTTP API's grouped format with structured sub-objects for typo tolerance, faceting, and pagination. All 22 settings from `meilisearch-types::Settings` that are relevant to embedded mode are included.

```rust
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::num::NonZeroUsize;
use serde::{Deserialize, Serialize};

/// Index settings configuration matching the Meilisearch HTTP API shape.
///
/// All fields are `Option` -- only set fields are applied on update.
/// Uses `camelCase` serialization for API shape parity.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Fields displayed in returned documents. `["*"]` or `None` = all fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub displayed_attributes: Option<Vec<String>>,

    /// Fields to search in, ordered by importance. `["*"]` = all fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub searchable_attributes: Option<Vec<String>>,

    /// Attributes usable for filtering and faceting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filterable_attributes: Option<Vec<String>>,

    /// Attributes usable for sorting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sortable_attributes: Option<BTreeSet<String>>,

    /// Ranking rules in order of importance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranking_rules: Option<Vec<String>>,

    /// Words ignored in search queries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_words: Option<BTreeSet<String>>,

    /// Characters that do NOT delimit term boundaries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub non_separator_tokens: Option<BTreeSet<String>>,

    /// Characters that delimit term boundaries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub separator_tokens: Option<BTreeSet<String>>,

    /// Strings parsed as single terms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dictionary: Option<BTreeSet<String>>,

    /// Synonym mappings: word -> list of synonyms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synonyms: Option<BTreeMap<String, Vec<String>>>,

    /// Attribute used for document deduplication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distinct_attribute: Option<String>,

    /// Precision level for the proximity ranking rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proximity_precision: Option<ProximityPrecision>,

    /// Typo tolerance configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typo_tolerance: Option<TypoToleranceSettings>,

    /// Faceting configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faceting: Option<FacetingSettings>,

    /// Pagination configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination: Option<PaginationSettings>,

    /// Embedder configurations for vector/semantic search, keyed by embedder name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedders: Option<HashMap<String, EmbedderSettings>>,

    /// Maximum duration of a search query in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_cutoff_ms: Option<u64>,

    /// Localized attributes rules for multilingual content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub localized_attributes: Option<Vec<LocalizedAttributeRule>>,

    /// Whether facet search is enabled. Default: true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facet_search: Option<bool>,

    /// Prefix search behavior: `"indexingTime"` (default) or `"disabled"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix_search: Option<String>,
}
```

#### 3.6.1 Settings Sub-Types

```rust
/// Typo tolerance configuration.
///
/// Matches the `typoTolerance` object in the HTTP API.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypoToleranceSettings {
    /// Whether typo tolerance is enabled. Default: true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,

    /// Minimum word length settings for typos.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_word_size_for_typos: Option<MinWordSizeForTypos>,

    /// Words for which typo tolerance is disabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_on_words: Option<BTreeSet<String>>,

    /// Attributes for which typo tolerance is disabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_on_attributes: Option<BTreeSet<String>>,

    /// Whether typo tolerance is disabled on numeric tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_on_numbers: Option<bool>,
}

/// Minimum word length before typos are allowed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinWordSizeForTypos {
    /// Minimum word length for one typo. Default: 5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub one_typo: Option<u8>,

    /// Minimum word length for two typos. Default: 9.
    /// Must be >= `one_typo`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub two_typos: Option<u8>,
}

/// Faceting configuration.
///
/// Matches the `faceting` object in the HTTP API.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FacetingSettings {
    /// Maximum number of facet values returned per facet. Default: 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_values_per_facet: Option<usize>,

    /// How to sort facet values per facet attribute.
    /// Key = attribute name (or `"*"` for default), value = `"alpha"` or `"count"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_facet_values_by: Option<BTreeMap<String, FacetValuesSort>>,
}

/// Facet value sort order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FacetValuesSort {
    /// Sort alphabetically.
    Alpha,
    /// Sort by count (descending).
    Count,
}

/// Pagination configuration.
///
/// Matches the `pagination` object in the HTTP API.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginationSettings {
    /// Maximum total hits that can be browsed. Default: 1000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_total_hits: Option<usize>,
}

/// Proximity precision for the proximity ranking rule.
///
/// Re-exports the milli type for direct use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProximityPrecision {
    ByWord,
    ByAttribute,
}

impl From<ProximityPrecision> for milli::proximity::ProximityPrecision {
    fn from(p: ProximityPrecision) -> Self {
        match p {
            ProximityPrecision::ByWord => Self::ByWord,
            ProximityPrecision::ByAttribute => Self::ByAttribute,
        }
    }
}

impl From<milli::proximity::ProximityPrecision> for ProximityPrecision {
    fn from(p: milli::proximity::ProximityPrecision) -> Self {
        match p {
            milli::proximity::ProximityPrecision::ByWord => Self::ByWord,
            milli::proximity::ProximityPrecision::ByAttribute => Self::ByAttribute,
        }
    }
}

/// Rule associating locales with attribute patterns.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalizedAttributeRule {
    /// Attribute name patterns (glob-style, e.g. `"*_ja"`).
    pub attribute_patterns: Vec<String>,
    /// ISO 639-3 locale codes (e.g. `["jpn", "cmn"]`).
    pub locales: Vec<String>,
}
```

#### 3.6.2 Individual Setting Accessors

All 20 individual settings from the HTTP API are exposed as `get_*/update_*/reset_*` triplets on `Index`:

```rust
impl Index {
    // -- Searchable Attributes --
    pub fn get_searchable_attributes(&self) -> Result<Vec<String>>;
    pub fn update_searchable_attributes(&self, attrs: Vec<String>) -> Result<()>;
    pub fn reset_searchable_attributes(&self) -> Result<()>;

    // -- Displayed Attributes --
    pub fn get_displayed_attributes(&self) -> Result<Option<Vec<String>>>;
    pub fn update_displayed_attributes(&self, attrs: Vec<String>) -> Result<()>;
    pub fn reset_displayed_attributes(&self) -> Result<()>;

    // -- Filterable Attributes --
    pub fn get_filterable_attributes(&self) -> Result<Vec<String>>;
    pub fn update_filterable_attributes(&self, attrs: Vec<String>) -> Result<()>;
    pub fn reset_filterable_attributes(&self) -> Result<()>;

    // -- Sortable Attributes --
    pub fn get_sortable_attributes(&self) -> Result<BTreeSet<String>>;
    pub fn update_sortable_attributes(&self, attrs: BTreeSet<String>) -> Result<()>;
    pub fn reset_sortable_attributes(&self) -> Result<()>;

    // -- Ranking Rules --
    pub fn get_ranking_rules(&self) -> Result<Vec<String>>;
    pub fn update_ranking_rules(&self, rules: Vec<String>) -> Result<()>;
    pub fn reset_ranking_rules(&self) -> Result<()>;

    // -- Stop Words --
    pub fn get_stop_words(&self) -> Result<Option<BTreeSet<String>>>;
    pub fn update_stop_words(&self, words: BTreeSet<String>) -> Result<()>;
    pub fn reset_stop_words(&self) -> Result<()>;

    // -- Non-Separator Tokens --
    pub fn get_non_separator_tokens(&self) -> Result<Option<BTreeSet<String>>>;
    pub fn update_non_separator_tokens(&self, tokens: BTreeSet<String>) -> Result<()>;
    pub fn reset_non_separator_tokens(&self) -> Result<()>;

    // -- Separator Tokens --
    pub fn get_separator_tokens(&self) -> Result<Option<BTreeSet<String>>>;
    pub fn update_separator_tokens(&self, tokens: BTreeSet<String>) -> Result<()>;
    pub fn reset_separator_tokens(&self) -> Result<()>;

    // -- Dictionary --
    pub fn get_dictionary(&self) -> Result<Option<BTreeSet<String>>>;
    pub fn update_dictionary(&self, words: BTreeSet<String>) -> Result<()>;
    pub fn reset_dictionary(&self) -> Result<()>;

    // -- Synonyms --
    pub fn get_synonyms(&self) -> Result<Option<BTreeMap<String, Vec<String>>>>;
    pub fn update_synonyms(&self, synonyms: BTreeMap<String, Vec<String>>) -> Result<()>;
    pub fn reset_synonyms(&self) -> Result<()>;

    // -- Distinct Attribute --
    pub fn get_distinct_attribute(&self) -> Result<Option<String>>;
    pub fn update_distinct_attribute(&self, attr: String) -> Result<()>;
    pub fn reset_distinct_attribute(&self) -> Result<()>;

    // -- Proximity Precision --
    pub fn get_proximity_precision(&self) -> Result<ProximityPrecision>;
    pub fn update_proximity_precision(&self, precision: ProximityPrecision) -> Result<()>;
    pub fn reset_proximity_precision(&self) -> Result<()>;

    // -- Typo Tolerance --
    pub fn get_typo_tolerance(&self) -> Result<TypoToleranceSettings>;
    pub fn update_typo_tolerance(&self, settings: TypoToleranceSettings) -> Result<()>;
    pub fn reset_typo_tolerance(&self) -> Result<()>;

    // -- Faceting --
    pub fn get_faceting(&self) -> Result<FacetingSettings>;
    pub fn update_faceting(&self, settings: FacetingSettings) -> Result<()>;
    pub fn reset_faceting(&self) -> Result<()>;

    // -- Pagination --
    pub fn get_pagination(&self) -> Result<PaginationSettings>;
    pub fn update_pagination(&self, settings: PaginationSettings) -> Result<()>;
    pub fn reset_pagination(&self) -> Result<()>;

    // -- Embedders --
    pub fn get_embedders(&self) -> Result<Option<HashMap<String, EmbedderSettings>>>;
    pub fn update_embedders(&self, embedders: HashMap<String, EmbedderSettings>) -> Result<()>;
    pub fn reset_embedders(&self) -> Result<()>;

    // -- Search Cutoff --
    pub fn get_search_cutoff_ms(&self) -> Result<Option<u64>>;
    pub fn update_search_cutoff_ms(&self, ms: u64) -> Result<()>;
    pub fn reset_search_cutoff_ms(&self) -> Result<()>;

    // -- Localized Attributes --
    pub fn get_localized_attributes(&self) -> Result<Option<Vec<LocalizedAttributeRule>>>;
    pub fn update_localized_attributes(
        &self,
        rules: Vec<LocalizedAttributeRule>,
    ) -> Result<()>;
    pub fn reset_localized_attributes(&self) -> Result<()>;

    // -- Facet Search --
    pub fn get_facet_search(&self) -> Result<bool>;
    pub fn update_facet_search(&self, enabled: bool) -> Result<()>;
    pub fn reset_facet_search(&self) -> Result<()>;

    // -- Prefix Search --
    pub fn get_prefix_search(&self) -> Result<String>;
    pub fn update_prefix_search(&self, mode: String) -> Result<()>;
    pub fn reset_prefix_search(&self) -> Result<()>;
}
```

**Implementation pattern:** Each `get_*` opens a read transaction, reads the setting from the milli index, converts to the lib type, and returns. Each `update_*` opens a write transaction, creates a `milli::update::Settings` builder, calls the corresponding `set_*` method, executes, and commits. Each `reset_*` is the same but calls `reset_*` on the builder.

---

### 3.7 Enhanced Index Operations

#### 3.7.1 Update Primary Key

```rust
impl Index {
    /// Update the primary key of an index.
    ///
    /// Can only be called when the index is empty (has no documents).
    /// Returns an error if the index already contains documents.
    pub fn update_primary_key(&self, primary_key: &str) -> Result<()>;
}
```

#### 3.7.2 Swap Indexes

```rust
impl Meilisearch {
    /// Atomically swap the contents of pairs of indexes.
    ///
    /// Each pair `(a, b)` causes index A to contain what was in B and vice versa.
    /// All settings, documents, and embedder configurations are swapped.
    ///
    /// # Errors
    ///
    /// - `IndexNotFound` if either index in a pair does not exist.
    /// - `IndexInUse` if either index has outstanding references.
    pub fn swap_indexes(&self, swaps: &[(&str, &str)]) -> Result<()>;
}
```

**Implementation:** Lock both indexes, rename the underlying LMDB data directories on the filesystem, update the in-memory `indexes` HashMap entries.

---

### 3.8 Instance Operations

```rust
/// Health status of the embedded instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    /// Always `"available"` if the call succeeds.
    pub status: String,
}

/// Version information for the embedded Meilisearch engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    /// Git commit SHA of the milli build.
    pub commit_sha: String,
    /// Git commit date.
    pub commit_date: String,
    /// Meilisearch package version (e.g. `"1.15.0"`).
    pub pkg_version: String,
}

/// Aggregate statistics across all indexes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalStats {
    /// Total database size in bytes.
    pub database_size: u64,
    /// ISO 8601 timestamp of the last index update, or `None` if never updated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_update: Option<String>,
    /// Per-index statistics, keyed by index UID.
    pub indexes: HashMap<String, IndexStats>,
}

impl Meilisearch {
    /// Check if the embedded instance is operational.
    ///
    /// For an embedded instance this always succeeds (returns `HealthStatus`).
    /// It exists for API parity with the HTTP SDK.
    pub fn health(&self) -> HealthStatus {
        HealthStatus { status: "available".to_string() }
    }

    /// Get version information for the embedded Meilisearch engine.
    ///
    /// Returns the milli engine version compiled into the binary.
    pub fn version(&self) -> VersionInfo;

    /// Get aggregate statistics across all indexes.
    pub fn stats(&self) -> Result<GlobalStats>;
}
```

**Implementation:** `version()` reads from `meilisearch_types::versioning::{VERSION_MAJOR, VERSION_MINOR, VERSION_PATCH}`. `stats()` iterates `list_indexes()`, calls `index_stats()` on each, and sums `database_size` from LMDB environment info.

---

### 3.9 Backup Operations

```rust
/// Information about a dump operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpInfo {
    /// Unique identifier for the dump.
    pub uid: String,
    /// Filesystem path where the dump was written.
    pub path: String,
}

impl Meilisearch {
    /// Create a database dump at the specified directory.
    ///
    /// The dump contains all indexes, documents, and settings in a
    /// portable format that can be imported by any Meilisearch instance.
    ///
    /// Returns the dump UID and output path.
    pub fn create_dump(&self, dump_dir: &std::path::Path) -> Result<DumpInfo>;

    /// Create a snapshot of the database at the specified directory.
    ///
    /// A snapshot is a byte-for-byte copy of the LMDB data files.
    /// It is faster than a dump but only compatible with the same
    /// Meilisearch/milli version.
    pub fn create_snapshot(&self, snapshot_dir: &std::path::Path) -> Result<()>;
}
```

**Implementation:** Dump creation iterates all indexes, serializes their settings and documents to JSON files in the Meilisearch dump format. Snapshot creation copies the LMDB `data.mdb` files from each index directory.

---

### 3.10 Experimental Features

```rust
/// Runtime-togglable experimental feature flags.
///
/// Matches the `RuntimeTogglableFeatures` from `meilisearch-types`.
/// Only features that are meaningful for embedded mode are included.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalFeatures {
    /// Enable metrics endpoint. Default: false.
    #[serde(default)]
    pub metrics: bool,

    /// Enable the logs route. Default: false.
    #[serde(default)]
    pub logs_route: bool,

    /// Enable editing documents by function. Default: false.
    #[serde(default)]
    pub edit_documents_by_function: bool,

    /// Enable the `CONTAINS` filter operator. Default: false.
    #[serde(default)]
    pub contains_filter: bool,

    /// Enable composite embedders. Default: false.
    #[serde(default)]
    pub composite_embedders: bool,

    /// Enable multimodal search. Default: false.
    #[serde(default)]
    pub multimodal: bool,

    /// Enable the vector store backend setting. Default: false.
    #[serde(default)]
    pub vector_store_setting: bool,
}

impl Meilisearch {
    /// Get the current experimental feature flags.
    pub fn get_experimental_features(&self) -> ExperimentalFeatures;

    /// Update experimental feature flags.
    ///
    /// Only fields that differ from the current value are changed.
    /// Returns the resulting feature flags after the update.
    pub fn update_experimental_features(
        &self,
        features: ExperimentalFeatures,
    ) -> ExperimentalFeatures;
}
```

**Implementation note:** For embedded mode, feature flags are stored in-memory on the `Meilisearch` struct. Some flags (like `contains_filter`) affect milli's search behavior and need to be propagated to the appropriate milli APIs when executing searches.

---

### 3.11 Extended Error Types

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    // -- Existing errors (unchanged) --
    #[error("Milli error: {0}")]
    Milli(#[from] milli::Error),

    #[error("Heed error: {0}")]
    Heed(#[from] milli::heed::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serde JSON error: {0}")]
    SerdeJson(#[from] serde_json::Error),

    #[error("Index not found: {0}")]
    IndexNotFound(String),

    #[error("Index already exists: {0}")]
    IndexAlreadyExists(String),

    #[error("Invalid index UID: {0}")]
    InvalidIndexUid(String),

    #[error("Index is in use and cannot be deleted: {0}")]
    IndexInUse(String),

    #[error("Internal error: {0}")]
    Internal(String),

    // -- New errors for SDK parity --

    /// A document with the requested ID was not found.
    #[error("Document not found: {0}")]
    DocumentNotFound(String),

    /// The primary key cannot be changed because the index has documents.
    #[error("Primary key already present; cannot change on a non-empty index")]
    PrimaryKeyAlreadyPresent,

    /// A primary key is required but the index has none and it cannot be inferred.
    #[error("Primary key is required but was not provided and could not be inferred")]
    PrimaryKeyRequired,

    /// An invalid filter expression was provided.
    #[error("Invalid filter expression: {0}")]
    InvalidFilter(String),

    /// An invalid sort expression was provided.
    #[error("Invalid sort expression: {0}")]
    InvalidSort(String),

    /// The requested embedder does not exist in settings.
    #[error("Embedder not found: {0}")]
    EmbedderNotFound(String),

    /// Dump creation failed.
    #[error("Dump creation failed: {0}")]
    DumpFailed(String),

    /// Snapshot creation failed.
    #[error("Snapshot creation failed: {0}")]
    SnapshotFailed(String),

    /// An experimental feature was used but is not enabled.
    #[error("Experimental feature not enabled: {0}")]
    ExperimentalFeatureNotEnabled(String),
}

pub type Result<T> = std::result::Result<T, Error>;
```

---

## 4. Testing Strategy

### 4.1 Unit Tests

Each module has targeted unit tests for type construction, serialization, and conversion:

| Module | Tests |
|--------|-------|
| `search.rs` | `SearchQuery` builder, `HitsInfo` serde round-trip, `MatchingStrategy` conversion |
| `settings.rs` | `Settings` builder, `TypoToleranceSettings` serde, `ProximityPrecision` conversion, `EmbedderSettings` factory methods |
| `error.rs` | `Error` display strings, `From` conversions |

### 4.2 Integration Tests

Integration tests use `tempfile::TempDir` for isolated LMDB environments. Each test creates a fresh `Meilisearch` instance.

```
tests/
+-- common/
|   +-- mod.rs              # Test helpers, fixtures, sample data
+-- index_operations.rs      # create, get, list, delete, swap, update_primary_key
+-- document_operations.rs   # add, update, get_one, get_many, get_with_opts,
|                            # delete_one, delete_many, delete_by_filter, clear
+-- search_basic.rs          # keyword search with all 28 parameters
+-- search_hybrid.rs         # hybrid search, semantic_hit_count
+-- search_multi.rs          # multi_search (non-federated and federated)
+-- search_facet.rs          # facet_search
+-- search_similar.rs        # get_similar_documents
+-- settings_bulk.rs         # get/update/reset settings (bulk)
+-- settings_individual.rs   # 20x get_*/update_*/reset_* triplets
+-- instance_info.rs         # health, version, stats
+-- backup.rs                # create_dump, create_snapshot
+-- experimental_features.rs # get/update experimental features
```

### 4.3 Test Fixtures

```rust
// tests/common/mod.rs

use wilysearch::{Meilisearch, MeilisearchOptions, Index, Settings};
use serde_json::json;
use std::sync::Arc;
use tempfile::TempDir;

/// Create a Meilisearch instance with a temporary database directory.
pub fn create_test_meili() -> (Meilisearch, TempDir) {
    let dir = TempDir::new().unwrap();
    let options = MeilisearchOptions {
        db_path: dir.path().to_path_buf(),
        ..Default::default()
    };
    let meili = Meilisearch::new(options).unwrap();
    (meili, dir)
}

/// Create a test index pre-loaded with movie documents.
pub fn create_movies_index(meili: &Meilisearch) -> Arc<Index> {
    let index = meili.create_index("movies", Some("id")).unwrap();
    index.add_documents(sample_movies(), None).unwrap();

    // Configure filterable/sortable for search tests
    let settings = Settings::new()
        .with_filterable_attributes(vec![
            "genre".into(), "year".into(), "rating".into(),
        ])
        .with_sortable_attributes(["year".into(), "rating".into()].into());
    index.update_settings(&settings).unwrap();

    index
}

pub fn sample_movies() -> Vec<serde_json::Value> {
    vec![
        json!({"id": 1, "title": "The Matrix", "year": 1999,
               "genre": ["sci-fi", "action"], "rating": 8.7}),
        json!({"id": 2, "title": "Inception", "year": 2010,
               "genre": ["sci-fi", "thriller"], "rating": 8.8}),
        json!({"id": 3, "title": "Interstellar", "year": 2014,
               "genre": ["sci-fi", "drama"], "rating": 8.6}),
        json!({"id": 4, "title": "The Dark Knight", "year": 2008,
               "genre": ["action", "crime", "drama"], "rating": 9.0}),
        json!({"id": 5, "title": "Pulp Fiction", "year": 1994,
               "genre": ["crime", "drama"], "rating": 8.9}),
    ]
}
```

### 4.4 Serialization Conformance Tests

Dedicated tests verify that types serialize to JSON matching the HTTP API shape:

```rust
#[test]
fn search_result_json_shape_offset_limit() {
    let result = SearchResult { /* offset/limit mode */ };
    let json = serde_json::to_value(&result).unwrap();
    assert!(json.get("estimatedTotalHits").is_some());  // camelCase
    assert!(json.get("limit").is_some());
    assert!(json.get("offset").is_some());
    assert!(json.get("totalHits").is_none());  // not present in offset mode
}

#[test]
fn search_result_json_shape_pagination() {
    let result = SearchResult { /* page mode */ };
    let json = serde_json::to_value(&result).unwrap();
    assert!(json.get("totalHits").is_some());
    assert!(json.get("hitsPerPage").is_some());
    assert!(json.get("page").is_some());
    assert!(json.get("totalPages").is_some());
    assert!(json.get("estimatedTotalHits").is_none());  // not present in page mode
}
```

---

## 5. Migration Path

### Phase 1: Core Search Enhancements (High Priority)

1. **Enhanced `SearchQuery`**: Add all 28 fields with builder methods
2. **Enhanced `SearchResult`**: Add `HitsInfo` enum, facet distribution, facet stats, semantic hit count, query vector
3. **Enhanced `SearchHit`**: Add `formatted`, `matches_position`, `ranking_score_details`, `vectors`
4. **Search implementation**: Wire new parameters through to milli's `Search` builder, implement formatting via `MatcherBuilder`, implement sorting via `milli::AscDesc`, implement faceting via `milli::FacetDistribution`

**Estimated effort:** This is the largest phase. The `formatted` and `matches_position` fields require significant implementation work using milli's `MatcherBuilder` and `FormatOptions`.

### Phase 2: Settings Restructuring (High Priority)

1. Restructure `Settings` to use `TypoToleranceSettings`, `FacetingSettings`, `PaginationSettings` sub-objects
2. Add `non_separator_tokens`, `separator_tokens`, `dictionary`, `proximity_precision`, `localized_attributes`, `facet_search`, `prefix_search` fields
3. Implement all 20 individual setting accessor triplets (`get_*/update_*/reset_*`)
4. Update `SettingsApplier` to handle new structured sub-objects
5. Update `read_settings_from_index` for the new shape

**Estimated effort:** Moderate. Most of the milli APIs already exist; this is mostly mapping work.

### Phase 3: Advanced Search Operations (Medium Priority)

1. **Multi-search** (non-federated): Iterate queries, execute each on its target index, collect results
2. **Multi-search** (federated): Execute all queries, merge hits by weighted score, handle facets_by_index and merge_facets
3. **Facet search**: Delegate to `milli::SearchForFacetValues`
4. **Similar documents**: Look up reference document's embedding, execute vector search

**Estimated effort:** Multi-search is moderate. Federated merge logic is complex (weighted score ranking, cross-index deduplication). Facet search and similar are straightforward.

### Phase 4: Document & Index Operations (Medium Priority)

1. `update_documents()` (partial merge) using `IndexDocumentsMethod::UpdateDocuments`
2. `get_documents()` with `GetDocumentsOptions` (filter, sort, fields, IDs)
3. `update_primary_key()` on `Index`
4. `swap_indexes()` on `Meilisearch`

**Estimated effort:** Low-moderate. `update_documents` reuses the existing `add_documents` flow with a different method flag.

### Phase 5: Instance & Operations (Lower Priority)

1. Instance info: `health()`, `version()`, `stats()`
2. Backup: `create_dump()`, `create_snapshot()`
3. Experimental features: `get_experimental_features()`, `update_experimental_features()`
4. Extended error variants

**Estimated effort:** Low. These are mostly metadata operations.

### Phase 6: Polish

1. Comprehensive documentation with examples
2. Full test coverage (target: >90% line coverage)
3. Performance benchmarks (search latency, indexing throughput)
4. Examples directory with real-world usage patterns

---

## 6. Decision Log

### D-1: Synchronous vs Asynchronous Operations

**Context:** The Meilisearch HTTP API returns task objects for long-running operations (document indexing, settings updates). The embedded library currently executes everything synchronously.

**Options:**
1. Keep everything synchronous -- simpler API, no task tracking
2. Add optional async layer with task semantics -- matches HTTP SDK
3. Hybrid -- sync by default, async opt-in for batch operations

**Decision:** Option 1 -- Synchronous only

**Rationale:** Embedded operations complete within the caller's control flow. The HTTP SDK's task system exists to decouple request handling from background processing, which is unnecessary when there is no HTTP server. Users who need async behavior can wrap calls in their own `tokio::spawn_blocking()`.

### D-2: Index Handle vs Direct Methods

**Context:** Should document/search/settings operations live on the `Meilisearch` struct or on the `Index` struct?

**Decision:** Mixed -- management on `Meilisearch`, per-index operations on `Index` (current pattern).

**Rationale:** This matches the HTTP SDK pattern (`client.index("movies").search(...)`), provides clear separation of concerns, and allows per-index state (the milli `Index` handle).

### D-3: Individual Settings Accessors

**Context:** Whether to implement all 20 individual get/update/reset settings triplets.

**Decision:** Implement all 20.

**Rationale:** Full SDK parity with minimal overhead. Each accessor is a thin wrapper around a milli transaction + builder call.

### D-4: Settings Type Shape

**Context:** The current `Settings` has flat fields (`typo_tolerance_enabled`, `min_word_size_for_one_typo`, `min_word_size_for_two_typos`, `max_values_per_facet`, `pagination_max_total_hits`). The HTTP API groups these into nested objects (`typoTolerance`, `faceting`, `pagination`).

**Decision:** Restructure to match the HTTP API shape with nested sub-objects.

**Rationale:**
- Serialization parity -- `serde_json::to_string(&settings)` produces the same JSON shape as the HTTP API
- Easier to reason about -- related settings are co-located
- Individual setting accessors (`get_typo_tolerance()`) return the sub-object directly
- Existing code will need migration, but the builder pattern makes this ergonomic

**Migration impact:** The `SettingsApplier` must unpack sub-objects into milli builder calls. The `read_settings_from_index` function must assemble sub-objects from flat milli reads.

### D-5: SearchQuery Field Name -- `q` vs `query`

**Context:** The current `SearchQuery` uses `query: Option<String>`. The HTTP API uses `q`.

**Decision:** Use `q` (matching the HTTP API).

**Rationale:** Serialization parity. Builder methods can still use `.new("search terms")` for ergonomics. The `q` field is familiar to Meilisearch users.

**Migration impact:** Rename `query` to `q` in `SearchQuery`. Update all callsites. Add `#[serde(alias = "query")]` temporarily for backward compatibility if needed.

### D-6: HybridQuery Restructuring

**Context:** The current `HybridSearchQuery` is a separate top-level type that wraps `SearchQuery` with `vector` and `semantic_ratio`. The HTTP API puts `vector` on `SearchQuery` directly and uses a `hybrid: { semanticRatio, embedder }` sub-object.

**Decision:** Match the HTTP API structure -- `vector` and `hybrid` are fields on `SearchQuery`.

**Rationale:**
- Single `SearchQuery` type works for keyword, vector, and hybrid search
- `Index::search(&SearchQuery)` is the unified entry point
- The separate `HybridSearchQuery` and `HybridSearchResult` types are removed
- `HybridSearchResult` is replaced by `SearchResult` with a populated `semantic_hit_count` field

**Migration impact:** Users of `HybridSearchQuery` construct a `SearchQuery` with `vector` and `hybrid` fields instead. The `hybrid_search()` method can be deprecated in favor of `search()` detecting the hybrid configuration.

### D-7: Filter Type -- String vs Value

**Context:** The current library uses `filter: Option<String>`. The HTTP API accepts both strings and array-of-arrays syntax (`filter: Option<Value>`).

**Decision:** Use `Option<Value>` to support both syntaxes.

**Rationale:** Parity with the HTTP API. milli's `Filter::from_str` can parse string filters; array syntax needs conversion to a string first (this is what the HTTP server does).

### D-8: Backward Compatibility of Existing Public API

**Context:** This redesign changes the shape of `SearchQuery`, `SearchResult`, `SearchHit`, `Settings`, and removes `HybridSearchQuery`/`HybridSearchResult`.

**Decision:** This is a breaking change. Bump the crate version accordingly (0.1.x -> 0.2.0).

**Rationale:** The library is pre-1.0 and actively developed. The changes are necessary for SDK parity and the current user base is small enough that a clean break is preferable to compatibility shims.
