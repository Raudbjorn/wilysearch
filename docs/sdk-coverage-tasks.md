# Meilisearch SDK Coverage Tasks

## Overview

Actionable task breakdown for achieving full Meilisearch SDK coverage in `meilisearch-lib`. Every task references requirement IDs from `sdk-coverage-requirements.md`, identifies files to modify, states completion criteria, and targets 2-4 hours of work.

**Current state (verified by codebase analysis):**

| Component | Implemented | Missing |
|-----------|-------------|---------|
| `Meilisearch` (facade) | `new`, `create_index`, `get_index`, `delete_index`, `list_indexes`, `index_exists`, `index_stats`, `with_vector_store` | `swap_indexes`, `multi_search`, `multi_search_federated`, `health`, `version`, `stats`, `create_dump`, `create_snapshot`, `get_experimental_features`, `update_experimental_features` |
| `Index` (documents) | `add_documents`, `get_document`, `get_documents`, `delete_document`, `delete_documents`, `delete_by_filter`, `clear`, `document_count`, `primary_key` | `update_documents`, field filtering on `get_document`/`get_documents`, `GetDocumentsOptions` |
| `Index` (search) | `search`, `search_simple`, `hybrid_search`, `search_vectors` | sort, facets, highlight/crop, match positions, ranking score details, matching strategy, distinct, page-based pagination, `attributesToSearchOn`, `locales`, `retrieveVectors`, `rankingScoreThreshold`, `facet_search`, `get_similar_documents` |
| `Index` (settings) | `get_settings`, `update_settings`, `reset_settings` (bulk only) | All 20 individual get/set/reset accessors; 7 missing settings fields on `Settings` struct |
| `SearchQuery` | `query`, `limit`, `offset`, `filter`, `attributes_to_retrieve`, `show_ranking_score` | 18 fields (see Phase 1) |
| `SearchResult` | `hits`, `query`, `processing_time_ms`, `estimated_total_hits`, `limit`, `offset` | `facet_distribution`, `facet_stats`, `semantic_hit_count`, page-based pagination fields |
| `SearchHit` | `document`, `ranking_score` | `formatted`, `matches_position`, `ranking_score_details`, `semantic_score` |
| `Settings` | 15 fields (searchable/filterable/sortable/displayed attrs, ranking rules, stop words, synonyms, embedders, distinct attr, typo tolerance enabled, min word sizes, max facet values, pagination max, search cutoff) | `non_separator_tokens`, `separator_tokens`, `dictionary`, `proximity_precision`, `localized_attributes`, `facet_search`, `prefix_search` |
| `Error` | `Milli`, `Heed`, `Io`, `SerdeJson`, `IndexNotFound`, `IndexAlreadyExists`, `InvalidIndexUid`, `IndexInUse`, `Internal` | Search/filter/sort/embedder/document/backup errors |
| Preprocessing | Fully implemented (66 tests) | -- |
| RAG | Traits + implementations | -- |
| Vector | `VectorStore` trait, `NoOpVectorStore`, SurrealDB | -- |

---

## Phase 1: Core Search Enhancements

**Goal:** Make `SearchQuery` / `SearchResult` / `SearchHit` feature-complete. This is the highest-value work for any consumer including TTTRPS.

### Task 1.1: Add sort, distinct, matchingStrategy, rankingScoreThreshold to SearchQuery

**Effort:** 3h | **Deps:** None | **Reqs:** US-3.3, US-3.1

**Files:**
- `src/search.rs` -- add fields + builder methods
- `src/index.rs` -- wire into `search()` via milli `sort_criteria()`, `distinct()`, `terms_matching_strategy()`, `ranking_score_threshold()`

**Fields to add to `SearchQuery`:**
```
sort: Option<Vec<String>>
distinct: Option<String>
matching_strategy: Option<String>  // "all" | "last" | "frequency"
ranking_score_threshold: Option<f64>
```

**Completion criteria:**
- [x] Fields added with serde annotations and builder methods
- [x] `search()` parses sort strings into `AscDesc` via `milli::AscDesc::from_str`
- [x] `search()` applies `terms_matching_strategy` mapping `"all"` -> `TermsMatchingStrategy::All`, `"last"` -> `Last`, `"frequency"` -> `Frequency`
- [x] `search()` applies `ranking_score_threshold` and `distinct`
- [x] Tests for sorted search (requires sortable attributes configured)

---

### Task 1.2: Add page-based pagination to SearchQuery and SearchResult

**Effort:** 2h | **Deps:** None | **Reqs:** US-3.4

**Files:**
- `src/search.rs` -- add fields to both structs
- `src/index.rs` -- compute page-based fields in `search()`

**Fields to add to `SearchQuery`:**
```
page: Option<usize>
hits_per_page: Option<usize>
```

**Fields to add to `SearchResult`:**
```
total_hits: Option<usize>
total_pages: Option<usize>
page: Option<usize>
hits_per_page: Option<usize>
```

**Completion criteria:**
- [x] When `page`/`hits_per_page` are set, compute `offset = (page - 1) * hits_per_page` and use `exhaustive_number_hits(true)` on milli search
- [x] Populate `total_hits` from candidates bitmap length, compute `total_pages = ceil(total_hits / hits_per_page)`
- [x] Mutually exclusive with offset/limit style (return error if both set)
- [x] Tests for page-based pagination

---

### Task 1.3: Add facets support to SearchQuery and SearchResult

**Effort:** 3h | **Deps:** None | **Reqs:** US-3.5

**Files:**
- `src/search.rs` -- add `facets` field to `SearchQuery`, `facet_distribution` and `facet_stats` to `SearchResult`
- `src/index.rs` -- compute facet distribution using `milli::FacetDistribution`

**Fields to add to `SearchQuery`:**
```
facets: Option<Vec<String>>
```

**Fields to add to `SearchResult`:**
```
facet_distribution: Option<HashMap<String, HashMap<String, u64>>>
facet_stats: Option<HashMap<String, FacetStats>>
```

**New struct:**
```rust
pub struct FacetStats { pub min: f64, pub max: f64 }
```

**Completion criteria:**
- [x] After search execution, if `facets` is set, construct `milli::FacetDistribution` with candidates bitmap
- [x] Compute stats for numeric facet fields
- [x] Tests with filterable attributes configured and facets requested

---

### Task 1.4: Add highlighting and cropping support

**Effort:** 4h | **Deps:** None | **Reqs:** US-3.6

**Files:**
- `src/search.rs` -- add fields to `SearchQuery` and `SearchHit`
- `src/index.rs` -- use `milli::MatcherBuilder` for formatting

**Fields to add to `SearchQuery`:**
```
attributes_to_highlight: Option<Vec<String>>
highlight_pre_tag: Option<String>
highlight_post_tag: Option<String>
attributes_to_crop: Option<Vec<String>>
crop_length: Option<usize>
crop_marker: Option<String>
```

**Fields to add to `SearchHit`:**
```
#[serde(rename = "_formatted")]
formatted: Option<Value>
```

**Completion criteria:**
- [x] After retrieving documents, if highlight/crop fields set, construct `MatcherBuilder` from `SearchResult::matching_words`
- [x] Configure `crop_marker`, `highlight_prefix`, `highlight_suffix`
- [x] For each hit, build formatted version of requested fields using `FormatOptions { highlight, crop }`
- [x] Return as `_formatted` in the hit
- [x] Tests for highlighting and cropping

---

### Task 1.5: Add match positions and ranking score details

**Effort:** 3h | **Deps:** None | **Reqs:** US-3.1, US-3.6

**Files:**
- `src/search.rs` -- add fields and new structs
- `src/index.rs` -- extract from milli results

**Fields to add to `SearchQuery`:**
```
show_matches_position: bool
show_ranking_score_details: bool
```

**Fields to add to `SearchHit`:**
```
#[serde(rename = "_matchesPosition")]
matches_position: Option<HashMap<String, Vec<MatchPosition>>>
#[serde(rename = "_rankingScoreDetails")]
ranking_score_details: Option<Value>
```

**New structs:**
```rust
pub struct MatchPosition { pub start: usize, pub length: usize }
```

**Completion criteria:**
- [x] When `show_matches_position` is true, extract positions from `MatcherBuilder` for each field
- [x] When `show_ranking_score_details` is true, set `ScoringStrategy::Detailed` and serialize `ScoreDetails` from milli's `document_scores`
- [x] Tests for both features

---

### Task 1.6: Add remaining SearchQuery fields

**Effort:** 2h | **Deps:** 1.1-1.5 | **Reqs:** US-3.1

**Files:**
- `src/search.rs` -- add fields
- `src/index.rs` -- wire into milli search

**Fields to add to `SearchQuery`:**
```
attributes_to_search_on: Option<Vec<String>>
locales: Option<Vec<String>>
retrieve_vectors: bool
```

**Completion criteria:**
- [x] `attributes_to_search_on` maps to milli `searchable_attributes()`
- [x] `locales` maps to milli `locales()` using `charabia::Language` parsing
- [x] `retrieve_vectors` maps to milli `retrieve_vectors()`
- [x] Builder methods and tests

---

### Task 1.7: Unify hybrid search into main SearchQuery

**Effort:** 3h | **Deps:** 1.1-1.6 | **Reqs:** US-3.7, US-3.8

**Files:**
- `src/search.rs` -- add `HybridConfig` and `vector` to `SearchQuery`, add `semantic_hit_count` to `SearchResult`
- `src/index.rs` -- integrate hybrid logic into `search()`, keep `hybrid_search()` as compatibility wrapper

**Fields to add to `SearchQuery`:**
```
hybrid: Option<HybridConfig>
vector: Option<Vec<f32>>
```

**New struct:**
```rust
pub struct HybridConfig {
    pub semantic_ratio: f32,
    pub embedder: Option<String>,
}
```

**Fields to add to `SearchResult`:**
```
semantic_hit_count: Option<u32>
```

**Completion criteria:**
- [x] When `hybrid` is set on `SearchQuery`, `search()` performs hybrid logic (existing implementation from `hybrid_search()`)
- [x] When `vector` is set without `hybrid`, perform pure vector search
- [x] `HybridSearchQuery` remains but delegates to `search()` internally
- [x] Tests covering all combinations

---

## Phase 2: Document & Settings Enhancements

### Task 2.1: Implement update_documents (partial update)

**Effort:** 3h | **Deps:** None | **Reqs:** US-2.2

**Files:**
- `src/index.rs` -- add `update_documents()` method

**Signature:**
```rust
pub fn update_documents(&self, documents: Vec<Value>, primary_key: Option<&str>) -> Result<u64>
```

**Completion criteria:**
- [x] Uses `milli::update::IndexDocumentsConfig` with `update_method: UpdateDocuments` (not `ReplaceDocuments`)
- [x] When document doesn't exist, creates it (same as add)
- [x] When document exists, merges fields (existing fields not in update are preserved)
- [x] Returns number of documents processed
- [x] Tests: update existing doc partially, update nonexistent doc, verify merge behavior

---

### Task 2.2: Add field filtering to get_document and get_documents

**Effort:** 2h | **Deps:** None | **Reqs:** US-2.3, US-2.4

**Files:**
- `src/index.rs` -- modify `get_document()`, add `get_documents_with_options()`

**New struct:**
```rust
pub struct GetDocumentsOptions {
    pub fields: Option<Vec<String>>,
    pub filter: Option<String>,
    pub offset: usize,
    pub limit: usize,
}
```

**Completion criteria:**
- [x] `get_document(id, fields)` accepts optional `fields: Option<&[&str]>` parameter
- [x] When fields specified, only those fields appear in returned JSON
- [x] `get_documents_with_options(options)` applies filter (using `milli::Filter`), field selection, and pagination
- [x] Existing `get_documents(offset, limit)` remains for backward compatibility
- [x] Tests

---

### Task 2.3: Add missing fields to Settings struct

**Effort:** 2h | **Deps:** None | **Reqs:** US-4.4

**Files:**
- `src/settings.rs` -- add 7 missing fields, update `SettingsApplier::apply()`, update `read_settings_from_index()`

**Fields to add to `Settings`:**
```
non_separator_tokens: Option<BTreeSet<String>>
separator_tokens: Option<BTreeSet<String>>
dictionary: Option<BTreeSet<String>>
proximity_precision: Option<String>  // "byWord" | "byAttribute"
localized_attributes: Option<Vec<LocalizedAttributesRule>>
facet_search: Option<bool>
prefix_search: Option<String>  // "indexingTime" | "disabled"
```

**Completion criteria:**
- [x] Fields added with serde annotations and builder methods
- [x] `SettingsApplier::apply()` maps each field to milli's `set_non_separator_tokens()`, `set_separator_tokens()`, `set_dictionary()`, `set_proximity_precision()`, `set_localized_attributes_rules()`, `set_facet_search()`, `set_prefix_search()`
- [x] `read_settings_from_index()` reads each from milli index
- [x] `reset_settings()` resets these 7 additional fields
- [x] Settings round-trip test (write settings, read back, verify match)

---

### Task 2.4: Implement individual setting accessors (batch 1 - simple types)

**Effort:** 3h | **Deps:** 2.3 | **Reqs:** US-4.4

**Files:**
- `src/index.rs` -- add methods (each setting = get/update/reset = 3 methods)

**Settings in this batch (9 settings, 27 methods):**
1. `searchable_attributes` -> `Vec<String>`
2. `displayed_attributes` -> `Vec<String>`
3. `filterable_attributes` -> `Vec<String>`
4. `sortable_attributes` -> `HashSet<String>`
5. `ranking_rules` -> `Vec<String>`
6. `stop_words` -> `HashSet<String>`
7. `synonyms` -> `HashMap<String, Vec<String>>`
8. `distinct_attribute` -> `Option<String>`
9. `search_cutoff_ms` -> `Option<u64>`

**Pattern for each:**
```rust
pub fn get_<setting>(&self) -> Result<T> { /* read_txn + index accessor */ }
pub fn update_<setting>(&self, value: T) -> Result<()> { /* write_txn + Settings builder + execute */ }
pub fn reset_<setting>(&self) -> Result<()> { /* write_txn + Settings builder reset + execute */ }
```

**Completion criteria:**
- [x] All 27 methods implemented
- [x] Each update applies via milli's Settings builder with a single-field settings update
- [x] Each reset uses the corresponding `reset_*` on milli's Settings builder
- [x] Tests for at least 3 representative settings (get, update, reset cycle)

---

### Task 2.5: Implement individual setting accessors (batch 2 - complex types)

**Effort:** 3h | **Deps:** 2.3 | **Reqs:** US-4.4

**Files:**
- `src/index.rs` -- add methods
- `src/settings.rs` -- add supporting structs if not present

**Settings in this batch (11 settings, 33 methods):**
1. `typo_tolerance` -> `TypoToleranceSettings` (composite struct)
2. `faceting` -> `FacetingSettings` (composite struct)
3. `pagination` -> `PaginationSettings` (composite struct)
4. `non_separator_tokens` -> `BTreeSet<String>`
5. `separator_tokens` -> `BTreeSet<String>`
6. `dictionary` -> `BTreeSet<String>`
7. `proximity_precision` -> `ProximityPrecision` enum
8. `embedders` -> `HashMap<String, EmbedderSettings>`
9. `localized_attributes` -> `Vec<LocalizedAttributesRule>`
10. `facet_search` -> `bool`
11. `prefix_search` -> `PrefixSearch` enum

**New supporting structs:**
```rust
pub struct TypoToleranceSettings {
    pub enabled: bool,
    pub min_word_size_for_typos: MinWordSizeForTypos,
    pub disable_on_words: Vec<String>,
    pub disable_on_attributes: Vec<String>,
}
pub struct MinWordSizeForTypos { pub one_typo: u8, pub two_typos: u8 }
pub struct FacetingSettings {
    pub max_values_per_facet: usize,
    pub sort_facet_values_by: HashMap<String, FacetSort>,
}
pub enum FacetSort { Alpha, Count }
pub struct PaginationSettings { pub max_total_hits: usize }
pub enum ProximityPrecision { ByWord, ByAttribute }
pub enum PrefixSearch { IndexingTime, Disabled }
```

**Completion criteria:**
- [x] All 33 methods implemented
- [x] Composite structs properly serialize/deserialize
- [x] Tests for typo_tolerance and embedders (most complex)

---

## Phase 3: Advanced Search Operations

### Task 3.1: Implement facet search

**Effort:** 3h | **Deps:** 1.3 | **Reqs:** US-3.10

**Files:**
- `src/search.rs` -- add `FacetSearchQuery`, `FacetSearchResult`, `FacetHit` structs
- `src/index.rs` -- add `facet_search()` method

**Structs:**
```rust
pub struct FacetSearchQuery {
    pub facet_name: String,
    pub facet_query: Option<String>,
    pub filter: Option<String>,
    pub matching_strategy: Option<String>,
    pub q: Option<String>,  // search query to restrict universe
}
pub struct FacetSearchResult {
    pub facet_hits: Vec<FacetHit>,
    pub facet_query: Option<String>,
    pub processing_time_ms: u64,
}
pub struct FacetHit { pub value: String, pub count: u64 }
```

**Completion criteria:**
- [x] `facet_search()` validates the facet is in filterable attributes
- [x] Uses milli's facet infrastructure (filter universe, then search facet values)
- [x] Applies optional filter to narrow document universe first
- [x] Returns matching facet values with counts
- [x] Tests with configured filterable attributes and facet search

---

### Task 3.2: Implement multi-search (non-federated)

**Effort:** 3h | **Deps:** 1.7 | **Reqs:** US-3.9

**Files:**
- `src/search.rs` -- add `MultiSearchQuery` struct
- `src/meilisearch.rs` -- add `multi_search()` method

**Structs:**
```rust
pub struct MultiSearchQuery {
    pub index_uid: String,
    pub query: SearchQuery,
}
```

**Completion criteria:**
- [x] `multi_search()` takes `Vec<MultiSearchQuery>`
- [x] Resolves each `index_uid` to an `Index` via `get_index()`
- [x] Executes each search independently, collects results
- [x] If one query fails, returns error for that query (or fail-fast, document the choice)
- [x] Tests with multiple indexes

---

### Task 3.3: Implement federated multi-search

**Effort:** 4h | **Deps:** 3.2 | **Reqs:** US-3.9

**Files:**
- `src/search.rs` -- add `FederationConfig`, `FederatedSearchResult`, `FederatedHit`, `FederationInfo`
- `src/meilisearch.rs` -- add `multi_search_federated()` method

**Completion criteria:**
- [x] Executes all queries, merges hits by ranking score
- [x] Applies global `offset` and `limit` from `FederationConfig`
- [x] Each hit includes `_federation` metadata (source index, query position, weighted score)
- [x] Optionally computes `facets_by_index` and merged `facet_distribution`
- [x] Tests with multiple indexes and various federation configs

---

### Task 3.4: Implement similar documents

**Effort:** 3h | **Deps:** 1.7 | **Reqs:** US-3.11

**Files:**
- `src/search.rs` -- add `SimilarQuery` struct
- `src/index.rs` -- add `get_similar_documents()` method

**Completion criteria:**
- [x] Uses `milli::Similar` to find similar documents by vector
- [x] Requires embedder name to look up document's vector
- [x] Excludes source document from results
- [x] Applies optional filter and pagination
- [x] Returns standard `SearchResult`
- [x] Returns `EmbedderNotFound` if embedder not configured
- [x] Tests (requires embedder configuration)

---

## Phase 4: Index Operations

### Task 4.1: Implement update_primary_key

**Effort:** 2h | **Deps:** None | **Reqs:** US-1.5

**Files:**
- `src/index.rs` -- add `update_primary_key()` method

**Completion criteria:**
- [x] Checks document count is 0 (returns `PrimaryKeyAlreadyPresent` if documents exist)
- [x] Uses milli Settings builder to set new primary key
- [x] Tests: update empty index, attempt update non-empty index

---

### Task 4.2: Implement swap_indexes

**Effort:** 3h | **Deps:** None | **Reqs:** US-1.6

**Files:**
- `src/meilisearch.rs` -- add `swap_indexes()` method

**Completion criteria:**
- [x] `swap_indexes(swaps: &[(&str, &str)])` accepts list of index pairs
- [x] Validates both indexes exist (returns `IndexNotFound` otherwise)
- [x] Locks both indexes, closes LMDB environments
- [x] Renames underlying directories atomically (rename A -> tmp, B -> A, tmp -> B)
- [x] Reopens indexes and updates in-memory map
- [x] Handles partial failure with rollback
- [x] Tests: swap two populated indexes, verify contents exchanged

---

### Task 4.3: Enhance list_indexes with pagination and IndexInfo

**Effort:** 2h | **Deps:** None | **Reqs:** US-1.3

**Files:**
- `src/meilisearch.rs` -- modify `list_indexes()`, add `list_indexes_with_pagination()`, add `IndexInfo` struct

**New struct:**
```rust
pub struct IndexInfo {
    pub uid: String,
    pub primary_key: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}
```

**Completion criteria:**
- [x] `list_indexes_with_pagination(offset, limit)` returns `(Vec<IndexInfo>, total)`
- [x] Existing `list_indexes()` remains unchanged for backward compatibility
- [x] `IndexInfo` includes primary key (read from index)
- [x] Tests

---

## Phase 5: Instance & Backup Operations

### Task 5.1: Implement health check and version info

**Effort:** 2h | **Deps:** None | **Reqs:** US-7.1, US-7.2

**Files:**
- `src/meilisearch.rs` -- add `health()`, `version()` methods
- `src/search.rs` or new `src/types.rs` -- add `HealthStatus`, `VersionInfo` structs
- `build.rs` (new) -- embed git commit SHA and date at build time

**Completion criteria:**
- [x] `health()` attempts to read from LMDB env, returns `HealthStatus { status: "available" }` or error
- [x] `version()` returns `VersionInfo { pkg_version, commit_sha, commit_date }` using `env!("CARGO_PKG_VERSION")` and build-script values
- [x] Tests

---

### Task 5.2: Implement global stats

**Effort:** 2h | **Deps:** None | **Reqs:** US-7.3

**Files:**
- `src/meilisearch.rs` -- add `stats()` method

**New struct:**
```rust
pub struct GlobalStats {
    pub database_size: u64,
    pub last_update: Option<String>,
    pub indexes: HashMap<String, IndexStats>,
}
```

**Completion criteria:**
- [x] Computes `database_size` from LMDB environment real disk usage
- [x] Iterates all indexes, calls `index_stats()` for each
- [x] Tracks `last_update` (latest update across all indexes)
- [x] Tests

---

### Task 5.3: Implement dump and snapshot creation

**Effort:** 4h | **Deps:** None | **Reqs:** US-6.1, US-6.2

**Files:**
- `src/meilisearch.rs` -- add `create_dump()`, `create_snapshot()` methods
- `src/options.rs` -- add `dump_dir` and `snapshot_dir` to `MeilisearchOptions`

**Completion criteria:**
- [x] `create_dump()` exports all indexes (documents + settings) to a dump directory with unique UID
- [x] `create_snapshot()` uses LMDB `copy_to_path` for each index to snapshot directory
- [x] Creates directories if they don't exist
- [x] Returns `DumpInfo { uid, status, started_at, finished_at }`
- [x] Tests: create dump, verify files exist; create snapshot, verify files exist

---

### Task 5.4: Implement experimental features

**Effort:** 2h | **Deps:** None | **Reqs:** US-8.1, US-8.2

**Files:**
- `src/meilisearch.rs` -- add `get_experimental_features()`, `update_experimental_features()` methods
- `src/search.rs` or `src/types.rs` -- add `ExperimentalFeatures` struct

**New struct:**
```rust
pub struct ExperimentalFeatures {
    pub metrics: Option<bool>,
    pub logs_route: Option<bool>,
    pub edit_documents_by_function: Option<bool>,
    pub contains_filter: Option<bool>,
    pub network: Option<bool>,
    pub get_task_documents_route: Option<bool>,
    pub composite_embedders: Option<bool>,
    pub chat_completions: Option<bool>,
    pub multimodal: Option<bool>,
    pub vector_store_setting: Option<bool>,
}
```

**Completion criteria:**
- [x] Feature flags stored in `Meilisearch` struct (in-memory, not persisted -- matches embedded use case)
- [x] `get_experimental_features()` returns current state
- [x] `update_experimental_features()` merges provided values (None = keep current)
- [x] Tests

---

## Phase 6: Error Handling & Polish

### Task 6.1: Extend error types

**Effort:** 2h | **Deps:** 1.1-5.4 (best done after features) | **Reqs:** NFR-2

**Files:**
- `src/error.rs` -- add new variants

**New error variants:**
```rust
DocumentNotFound(String),
PrimaryKeyAlreadyPresent,
PrimaryKeyRequired,
InvalidFilter(String),
InvalidSort(String),
AttributeNotFilterable(String),
AttributeNotSortable(String),
FacetNotFilterable(String),
EmbedderNotFound(String),
EmbeddingFailed(String),
DumpCreationFailed(String),
SnapshotCreationFailed(String),
InvalidPagination(String),  // both offset/limit and page/hitsPerPage
```

**Completion criteria:**
- [x] All variants added
- [x] Used in appropriate places throughout codebase (replace `Error::Internal` where a specific error applies)
- [x] Error messages are descriptive and include the invalid value/context

---

### Task 6.2: Rustdoc on all public APIs

**Effort:** 4h | **Deps:** All feature tasks | **Reqs:** NFR-4

**Files:**
- `src/lib.rs`, `src/meilisearch.rs`, `src/index.rs`, `src/search.rs`, `src/settings.rs`, `src/error.rs`, `src/options.rs`

**Completion criteria:**
- [x] Every public struct, enum, trait, and function has `///` doc comments
- [x] At least 5 `# Example` blocks in doc comments (search, add documents, settings, multi-search, preprocessing)
- [x] Module-level `//!` documentation for each module
- [x] `cargo doc --no-deps` generates clean output with no warnings

---

### Task 6.3: Create examples directory

**Effort:** 3h | **Deps:** All feature tasks | **Reqs:** NFR-4

**Files:**
- `examples/basic_search.rs`
- `examples/hybrid_search.rs`
- `examples/multi_search.rs`
- `examples/settings.rs`
- `examples/preprocessing.rs`

**Completion criteria:**
- [x] Each example compiles and runs against a temp directory
- [x] Each demonstrates a distinct capability
- [x] Examples listed in crate-level docs

---

### Task 6.4: Integration test infrastructure

**Effort:** 3h | **Deps:** None (can start early) | **Reqs:** NFR-5

**Files:**
- `tests/common/mod.rs` -- shared fixtures and helpers
- `tests/integration.rs` -- test entrypoint

**Completion criteria:**
- [x] `TestContext` struct that creates temp dir, initializes `Meilisearch`, and cleans up on drop
- [x] `create_test_index(name, docs, settings)` helper
- [x] Sample document generators: `sample_movies()`, `sample_books()` with diverse field types
- [x] Works with `cargo test` -- no external dependencies

---

### Task 6.5: Comprehensive integration tests

**Effort:** 4h | **Deps:** 6.4, all features | **Reqs:** NFR-5

**Files:**
- `tests/search_tests.rs` -- basic, filtered, sorted, faceted, highlighted, hybrid, multi, facet search, similar
- `tests/document_tests.rs` -- add, update, get, delete, filter delete, clear, field filtering
- `tests/settings_tests.rs` -- bulk and individual get/update/reset
- `tests/index_tests.rs` -- create, get, list, delete, swap, update primary key, stats

**Completion criteria:**
- [x] At least 40 integration tests covering all major features
- [x] Tests exercise both happy paths and error paths
- [x] All tests pass with `cargo test`

---

## Dependency Graph

```
INDEPENDENT (can start in parallel):
  1.1 (sort/distinct/strategy)
  1.2 (page pagination)
  1.3 (facets)
  1.4 (highlight/crop)
  1.5 (match positions/score details)
  2.1 (update documents)
  2.2 (field filtering)
  2.3 (settings fields)
  4.1 (update primary key)
  4.2 (swap indexes)
  4.3 (list indexes enhanced)
  5.1 (health/version)
  5.2 (global stats)
  5.3 (dump/snapshot)
  5.4 (experimental features)
  6.4 (test infrastructure)

DEPENDS ON Phase 1 core:
  1.6 (remaining query fields)     <- 1.1-1.5
  1.7 (hybrid unification)         <- 1.1-1.6

DEPENDS ON 2.3:
  2.4 (individual settings batch 1) <- 2.3
  2.5 (individual settings batch 2) <- 2.3

DEPENDS ON Phase 1 completion:
  3.1 (facet search)               <- 1.3
  3.2 (multi-search)               <- 1.7
  3.4 (similar documents)          <- 1.7

DEPENDS ON 3.2:
  3.3 (federated multi-search)     <- 3.2

DEPENDS ON all features:
  6.1 (error types)                <- all features
  6.2 (rustdoc)                    <- all features
  6.3 (examples)                   <- all features
  6.5 (integration tests)          <- 6.4, all features
```

**Suggested parallelization (2 developers):**

```
Dev A (search focus):       Dev B (settings/ops focus):
  1.1 sort/distinct           2.1 update documents
  1.2 pagination              2.2 field filtering
  1.3 facets                  2.3 settings fields
  1.4 highlighting            2.4 individual settings batch 1
  1.5 match positions         2.5 individual settings batch 2
  1.6 remaining fields        4.1 update primary key
  1.7 hybrid unification      4.2 swap indexes
  3.1 facet search            4.3 list indexes enhanced
  3.2 multi-search            5.1 health/version
  3.3 federated search        5.2 global stats
  3.4 similar documents       5.3 dump/snapshot
                              5.4 experimental features
--- then together ---
  6.1 error types
  6.2 rustdoc
  6.3 examples
  6.4 test infrastructure
  6.5 integration tests
```

---

## Effort Summary

| Phase | Tasks | Est. Hours | Description |
|-------|-------|-----------|-------------|
| 1 | 7 | 20h | Core search enhancements |
| 2 | 5 | 13h | Document & settings |
| 3 | 4 | 13h | Advanced search |
| 4 | 3 | 7h | Index operations |
| 5 | 4 | 10h | Instance & backup |
| 6 | 5 | 16h | Error handling & polish |
| **Total** | **28** | **79h** | |

---

## Out of Scope

These are server-only features that belong in the `meilisearch` crate, not `meilisearch-lib`:

- **Network/federation** -- multi-node clustering
- **Chat completions** -- already handled in `meilisearch` crate's chat module
- **Webhooks** -- HTTP callback delivery
- **Logs stream** -- real-time log streaming
- **Metrics export** -- Prometheus metrics
- **Export** -- CSV/NDJSON/JSON export endpoint
- **API keys** -- authentication/authorization (embedded = trusted context)
- **Task queuing / batches** -- async task queue (embedded operations are synchronous)
- **Document edit by RHAI function** -- server-side RHAI scripting
- **CSV/NDJSON ingestion formats** -- library accepts `Vec<Value>` (callers handle format conversion)

---

## Success Criteria

1. **Feature parity:** All SDK search, document, settings, and index operations available
2. **Test coverage:** >80% on new code, 40+ integration tests
3. **Documentation:** All public APIs documented with rustdoc, 5 example programs
4. **Performance:** No regression in search/index benchmarks
5. **Thread safety:** All operations safe for `Send + Sync` concurrent access
6. **Error handling:** Specific error types for all failure modes (no `Internal("...")` for known errors)
7. **Backward compatibility:** Existing public API unchanged; all additions are additive
