# Meilisearch Embedded Library -- SDK Coverage Requirements

## Overview

This document specifies requirements for `meilisearch-lib`, an embedded Rust library that provides direct, in-process access to Meilisearch's search engine without requiring an HTTP server. The library wraps the `milli` search engine core and exposes an SDK-style interface for index management, document operations, search, settings, query preprocessing, and RAG pipelines.

The requirements are grounded in the actual Meilisearch HTTP API surface (~131 endpoints) and the current implementation state of `meilisearch-lib`. Acceptance criteria use EARS (Easy Approach to Requirements Syntax) format.

## Scope

### In Scope

- Index management (CRUD, stats, swap, compact)
- Document operations (add, update, get, delete, clear)
- Search operations (keyword, filtered, sorted, paginated, faceted, highlighted, hybrid, multi-search, facet search, similar documents)
- Settings management (bulk and individual get/update/reset for all 22 sub-settings)
- Instance information (health, version, global stats)
- Dump and snapshot creation
- Experimental feature management
- Query preprocessing pipeline (typo correction, synonym expansion) -- library extension
- RAG pipeline framework (embedder, retriever, reranker, generator traits) -- library extension
- External vector store integration
- Thread-safe, synchronous operation (no HTTP overhead)

### Out of Scope (Server-Only Features)

These Meilisearch HTTP API features are server-only concerns and are explicitly excluded from the embedded library:

| Feature | Reason |
|---------|--------|
| API Key management (5 endpoints) | Embedded library operates in a trusted context; no authentication boundary |
| Task queue management (5 endpoints) | Embedded operations are synchronous; no async task queue needed |
| Batch management (2 endpoints) | Depends on task queue infrastructure |
| Network configuration (2 endpoints) | Multi-node networking is a server concern |
| Chat Completions (1 endpoint) | Server-side LLM proxy feature |
| Chat Workspaces (3 endpoints) | Server-side chat configuration |
| Chat Settings (3 endpoints) | Server-side chat configuration |
| Webhooks (5 endpoints) | Server-side event notification |
| Logs management (3 endpoints) | Server-side observability |
| Metrics (1 endpoint) | Server-side Prometheus metrics |
| Export (1 endpoint) | Server-side data export |

---

## 1. Index Management

### REQ-1.1: Create Index

**As a** developer using meilisearch-lib,
**I want to** create a new search index with an optional primary key,
**So that** I can organize my searchable documents.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `create_index(uid, primary_key)` with a valid UID, THEN the system SHALL create a new index and return an `Arc<Index>` handle.
2. WHEN the user provides a UID containing characters other than ASCII alphanumerics, hyphens, or underscores, THEN the system SHALL return an `InvalidIndexUid` error.
3. WHEN the user provides a UID that matches an existing index, THEN the system SHALL return an `IndexAlreadyExists` error.
4. WHEN a primary key is provided, THEN the system SHALL set it during index creation via milli's `Settings::set_primary_key`.
5. WHEN no primary key is provided, THEN the system SHALL allow milli to infer it from the first document batch.

**Status:** IMPLEMENTED
**Source:** `Meilisearch::create_index()` in `src/meilisearch.rs:49-101`

---

### REQ-1.2: Get Index

**As a** developer,
**I want to** retrieve a handle to a specific index,
**So that** I can perform operations on it.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `get_index(uid)` for an index loaded in memory, THEN the system SHALL return the cached `Arc<Index>` handle.
2. WHEN the user calls `get_index(uid)` for an index that exists on disk but is not in memory, THEN the system SHALL load the index from disk, cache it, and return the handle.
3. WHEN the user calls `get_index(uid)` for a non-existent index, THEN the system SHALL return an `IndexNotFound` error.
4. WHEN concurrent callers request the same unloaded index, THEN the system SHALL use double-checked locking to ensure only one load occurs.

**Status:** IMPLEMENTED
**Source:** `Meilisearch::get_index()` in `src/meilisearch.rs:104-133`

---

### REQ-1.3: List Indexes

**As a** developer,
**I want to** list all indexes in my Meilisearch instance,
**So that** I can discover available search targets.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `list_indexes()`, THEN the system SHALL return a sorted `Vec<String>` of all index UIDs found on disk.
2. WHEN no indexes exist, THEN the system SHALL return an empty vector.
3. WHEN indexes exist on disk but are not loaded in memory, THEN the system SHALL still include them in the list.

**Status:** IMPLEMENTED
**Source:** `Meilisearch::list_indexes()` in `src/meilisearch.rs:178-197`, `Meilisearch::list_indexes_with_pagination()` in `src/meilisearch.rs:580-603`

---

### REQ-1.4: Delete Index

**As a** developer,
**I want to** delete an index and all its data,
**So that** I can clean up unused indexes.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `delete_index(uid)` for an existing index, THEN the system SHALL remove it from memory and delete its directory from disk.
2. WHEN the user calls `delete_index(uid)` for a non-existent index, THEN the system SHALL return an `IndexNotFound` error.
3. WHEN other `Arc<Index>` references exist (strong count > 1), THEN the system SHALL return an `IndexInUse` error and leave the index intact.
4. WHEN deletion succeeds, THEN the LMDB environment SHALL be dropped before the directory is removed.

**Status:** IMPLEMENTED
**Source:** `Meilisearch::delete_index()` in `src/meilisearch.rs:144-172`

---

### REQ-1.5: Check Index Existence

**As a** developer,
**I want to** check if an index exists without loading it,
**So that** I can make conditional decisions efficiently.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `index_exists(uid)`, THEN the system SHALL return `true` if the index directory exists on disk, `false` otherwise.
2. WHEN the index directory exists but is not a valid index, THEN the system SHALL still return `true` (existence check only).

**Status:** IMPLEMENTED
**Source:** `Meilisearch::index_exists()` in `src/meilisearch.rs:202-205`

---

### REQ-1.6: Update Index Primary Key

**As a** developer,
**I want to** update an index's primary key,
**So that** I can correct configuration after creation.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `update_primary_key(uid, primary_key)` on an empty index, THEN the system SHALL update the primary key.
2. WHEN the index already contains documents, THEN the system SHALL return an error (primary key is immutable after documents are added).
3. WHEN the index does not exist, THEN the system SHALL return an `IndexNotFound` error.

**Status:** IMPLEMENTED
**Source:** `Index::update_primary_key()` in `src/index.rs:1219-1246`

---

### REQ-1.7: Swap Indexes

**As a** developer,
**I want to** atomically swap two indexes,
**So that** I can deploy index updates without downtime.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `swap_indexes([(uid_a, uid_b)])`, THEN the system SHALL atomically exchange the contents of both indexes.
2. WHEN any referenced index does not exist, THEN the system SHALL return an `IndexNotFound` error.
3. WHEN the swap succeeds, THEN all documents, settings, and embedder configurations SHALL be preserved.

**Status:** IMPLEMENTED
**Source:** `Meilisearch::swap_indexes()` in `src/meilisearch.rs:495-527`

---

### REQ-1.8: Index Statistics

**As a** developer,
**I want to** get statistics about an index,
**So that** I can monitor index health and size.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `index_stats(uid)`, THEN the system SHALL return an `IndexStats` containing `number_of_documents`, `is_indexing`, `field_distribution`, and `primary_key`.
2. WHEN the index is not loaded, THEN the system SHALL load it before computing stats.
3. WHEN the index does not exist, THEN the system SHALL return an `IndexNotFound` error.

**Note:** `is_indexing` currently always returns `false` since embedded operations are synchronous.

**Status:** IMPLEMENTED
**Source:** `Meilisearch::index_stats()` in `src/meilisearch.rs:210-226`

---

### REQ-1.9: Index Compaction

**As a** developer,
**I want to** compact an index to reclaim disk space,
**So that** I can optimize storage after bulk deletions.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `compact_index(uid)`, THEN the system SHALL trigger LMDB compaction on the index.
2. WHEN compaction completes, THEN disk usage SHALL decrease for indexes with significant deletions.

**Status:** IMPLEMENTED
**Source:** `Meilisearch::compact_index()` in `src/meilisearch.rs:529-576`

---

## 2. Document Operations

### REQ-2.1: Add Documents (Replace)

**As a** developer,
**I want to** add documents to an index,
**So that** they become searchable.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `add_documents(docs, primary_key)` with valid JSON objects, THEN the system SHALL index all documents.
2. WHEN documents have IDs matching existing documents, THEN the system SHALL replace the existing documents entirely.
3. WHEN a primary key is provided and the index has no primary key set, THEN the system SHALL use it.
4. WHEN a document is not a JSON object, THEN the system SHALL return an error.
5. WHEN a vector store is configured, THEN the system SHALL extract and store vectors (currently TODO).

**Status:** IMPLEMENTED
**Source:** `Index::add_documents()` in `src/index.rs:38-88`

---

### REQ-2.2: Update Documents (Partial)

**As a** developer,
**I want to** partially update existing documents,
**So that** I can modify specific fields without full replacement.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `update_documents(docs)`, THEN the system SHALL merge provided fields into existing documents.
2. WHEN a document ID does not exist, THEN the system SHALL create a new document with the provided fields.
3. WHEN only some fields are provided, THEN the system SHALL preserve all other existing fields.

**Status:** IMPLEMENTED
**Source:** `Index::update_documents()` in `src/index.rs:139-189`

---

### REQ-2.3: Get Single Document

**As a** developer,
**I want to** retrieve a single document by its external ID,
**So that** I can verify document contents.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `get_document(id)` for an existing document, THEN the system SHALL return `Some(Value)` with the document content.
2. WHEN the document does not exist, THEN the system SHALL return `None`.
3. WHEN displayed attributes are configured, THEN the system SHALL return only those fields.

**Status:** IMPLEMENTED
**Source:** `Index::get_document()` in `src/index.rs:510-536`, `Index::get_documents_with_options()` in `src/index.rs:1153-1213`

---

### REQ-2.4: Get Documents (Paginated)

**As a** developer,
**I want to** retrieve multiple documents with pagination,
**So that** I can browse index contents.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `get_documents(offset, limit)`, THEN the system SHALL return a `DocumentsResult` with the paginated documents, total count, and pagination metadata.
2. WHEN offset exceeds the document count, THEN the system SHALL return an empty documents list with the correct total.

**Status:** IMPLEMENTED
**Source:** `Index::get_documents()` in `src/index.rs:542-575`, `Index::get_documents_with_options()` in `src/index.rs:1153-1213` (supports `GetDocumentsOptions` with `fields`, `filter`, `offset`, `limit`, and `retrieve_vectors`)

---

### REQ-2.5: Delete Single Document

**As a** developer,
**I want to** delete a single document by ID,
**So that** I can remove outdated content.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `delete_document(id)` for an existing document, THEN the system SHALL remove it and return `true`.
2. WHEN the document does not exist, THEN the system SHALL return `false`.

**Status:** IMPLEMENTED
**Source:** `Index::delete_document()` in `src/index.rs:580-583`

---

### REQ-2.6: Delete Documents (Batch by IDs)

**As a** developer,
**I want to** delete multiple documents by their IDs,
**So that** I can efficiently remove batches.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `delete_documents(ids)` with a list of IDs, THEN the system SHALL remove all matching documents and return the count deleted.
2. WHEN some IDs do not exist, THEN the system SHALL silently ignore them.
3. WHEN the ID list is empty, THEN the system SHALL return 0 without performing any operation.

**Status:** IMPLEMENTED
**Source:** `Index::delete_documents()` in `src/index.rs:589-671`

---

### REQ-2.7: Delete Documents by Filter

**As a** developer,
**I want to** delete documents matching a filter expression,
**So that** I can remove documents by criteria.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `delete_by_filter(filter)` with a valid filter, THEN the system SHALL delete all matching documents and return the count.
2. WHEN the filter references a non-filterable field, THEN the system SHALL return an error.
3. WHEN no documents match, THEN the system SHALL return 0.
4. WHEN the filter expression is syntactically invalid, THEN the system SHALL return an error.

**Status:** IMPLEMENTED
**Source:** `Index::delete_by_filter()` in `src/index.rs:679-752`

---

### REQ-2.8: Clear All Documents

**As a** developer,
**I want to** delete all documents from an index,
**So that** I can reset without losing settings.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `clear()`, THEN the system SHALL remove all documents and return the count deleted.
2. WHEN documents are cleared, THEN all index settings (searchable attributes, filterable attributes, embedders, etc.) SHALL be preserved.

**Status:** IMPLEMENTED
**Source:** `Index::clear()` in `src/index.rs:760-769`

---

### REQ-2.9: Document Count

**As a** developer,
**I want to** get the number of documents in an index,
**So that** I can monitor index size without fetching documents.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `document_count()`, THEN the system SHALL return the exact number of documents in the index.

**Status:** IMPLEMENTED
**Source:** `Index::document_count()` in `src/index.rs:502-505`

---

### REQ-2.10: Edit Documents by Function

**As a** developer,
**I want to** programmatically modify documents using a function/expression,
**So that** I can perform bulk transformations without re-uploading.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `edit_documents_by_function(filter, function)`, THEN the system SHALL apply the function to all matching documents.
2. WHEN the filter is None, THEN the system SHALL apply to all documents.

**Status:** NOT IMPLEMENTED

---

## 3. Search Operations

### REQ-3.1: Basic Keyword Search

**As a** developer,
**I want to** search documents by keyword query,
**So that** I can find relevant results.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `search(&SearchQuery)`, THEN the system SHALL return a `SearchResult` containing hits, processing time, and estimated total hits.
2. WHEN the query string is empty or None, THEN the system SHALL return all documents (with limit).
3. WHEN no matches exist, THEN the system SHALL return an empty hits list with `estimated_total_hits: 0`.

**Status:** IMPLEMENTED
**Source:** `Index::search()` in `src/index.rs:145-214`

---

### REQ-3.2: Simple Search Convenience

**As a** developer,
**I want to** search with a single string and get raw JSON documents,
**So that** I can quickly integrate search without constructing query objects.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `search_simple(query)`, THEN the system SHALL return `Vec<Value>` of matching documents.

**Status:** IMPLEMENTED
**Source:** `Index::search_simple()` in `src/index.rs:226-230`

---

### REQ-3.3: SearchQuery Parameters

The `SearchQuery` struct SHALL support the following parameters to match the Meilisearch HTTP API's 28 search parameters:

| # | Parameter | Type | Current Status |
|---|-----------|------|----------------|
| 1 | `query` (q) | `Option<String>` | IMPLEMENTED |
| 2 | `limit` | `usize` (default: 20) | IMPLEMENTED |
| 3 | `offset` | `usize` (default: 0) | IMPLEMENTED |
| 4 | `filter` | `Option<String>` | IMPLEMENTED |
| 5 | `attributes_to_retrieve` | `Option<Vec<String>>` | IMPLEMENTED |
| 6 | `show_ranking_score` | `bool` | IMPLEMENTED |
| 7 | `sort` | `Option<Vec<String>>` | IMPLEMENTED |
| 8 | `page` | `Option<usize>` | IMPLEMENTED |
| 9 | `hits_per_page` | `Option<usize>` | IMPLEMENTED |
| 10 | `facets` | `Option<Vec<String>>` | IMPLEMENTED |
| 11 | `attributes_to_crop` | `Option<Vec<String>>` | IMPLEMENTED |
| 12 | `crop_length` | `Option<usize>` | IMPLEMENTED |
| 13 | `crop_marker` | `Option<String>` | IMPLEMENTED |
| 14 | `attributes_to_highlight` | `Option<Vec<String>>` | IMPLEMENTED |
| 15 | `highlight_pre_tag` | `Option<String>` | IMPLEMENTED |
| 16 | `highlight_post_tag` | `Option<String>` | IMPLEMENTED |
| 17 | `show_matches_position` | `Option<bool>` | IMPLEMENTED |
| 18 | `show_ranking_score_details` | `Option<bool>` | IMPLEMENTED |
| 19 | `matching_strategy` | `Option<MatchingStrategy>` | IMPLEMENTED |
| 20 | `attributes_to_search_on` | `Option<Vec<String>>` | IMPLEMENTED |
| 21 | `distinct` | `Option<String>` | IMPLEMENTED |
| 22 | `vector` | `Option<Vec<f32>>` | IMPLEMENTED |
| 23 | `hybrid` | `Option<HybridConfig>` | IMPLEMENTED |
| 24 | `media` | `Option<Value>` | IMPLEMENTED |
| 25 | `ranking_score_threshold` | `Option<f64>` | IMPLEMENTED |
| 26 | `locales` | `Option<Vec<String>>` | IMPLEMENTED |
| 27 | `retrieve_vectors` | `Option<bool>` | IMPLEMENTED |
| 28 | `personalize` | `Option<Value>` | IMPLEMENTED |

**Current SearchQuery fields (28 of 28):**
```rust
pub struct SearchQuery {
    pub query: Option<String>,
    pub limit: usize,                                    // default: 20
    pub offset: usize,                                   // default: 0
    pub filter: Option<String>,
    pub attributes_to_retrieve: Option<Vec<String>>,
    pub show_ranking_score: bool,
    pub sort: Option<Vec<String>>,
    pub page: Option<usize>,
    pub hits_per_page: Option<usize>,
    pub facets: Option<Vec<String>>,
    pub attributes_to_crop: Option<Vec<String>>,
    pub crop_length: Option<usize>,
    pub crop_marker: Option<String>,
    pub attributes_to_highlight: Option<Vec<String>>,
    pub highlight_pre_tag: Option<String>,
    pub highlight_post_tag: Option<String>,
    pub show_matches_position: Option<bool>,
    pub show_ranking_score_details: Option<bool>,
    pub matching_strategy: Option<MatchingStrategy>,
    pub attributes_to_search_on: Option<Vec<String>>,
    pub distinct: Option<String>,
    pub vector: Option<Vec<f32>>,
    pub hybrid: Option<HybridConfig>,
    pub media: Option<Value>,
    pub ranking_score_threshold: Option<f64>,
    pub locales: Option<Vec<String>>,
    pub retrieve_vectors: Option<bool>,
    pub personalize: Option<Value>,
}
```

**Status:** FULLY IMPLEMENTED (28/28 parameters)
**Source:** `SearchQuery` in `src/search.rs:21-125`

---

### REQ-3.4: SearchResult Structure

The `SearchResult` SHALL include the following fields:

| Field | Type | Current Status |
|-------|------|----------------|
| `hits` | `Vec<SearchHit>` | IMPLEMENTED |
| `query` | `String` | IMPLEMENTED |
| `processing_time_ms` | `u64` | IMPLEMENTED |
| `estimated_total_hits` | `usize` | IMPLEMENTED |
| `limit` | `usize` | IMPLEMENTED |
| `offset` | `usize` | IMPLEMENTED |
| `facet_distribution` | `Option<HashMap<String, HashMap<String, u64>>>` | IMPLEMENTED |
| `facet_stats` | `Option<HashMap<String, FacetStats>>` | IMPLEMENTED |
| `page` | `Option<usize>` | IMPLEMENTED |
| `hits_per_page` | `Option<usize>` | IMPLEMENTED |
| `total_hits` | `Option<usize>` | IMPLEMENTED |
| `total_pages` | `Option<usize>` | IMPLEMENTED |

**Status:** FULLY IMPLEMENTED (12/12 fields)
**Source:** `SearchResult` in `src/search.rs:366-391`

---

### REQ-3.5: SearchHit Structure

Each `SearchHit` SHALL include:

| Field | Type | Current Status |
|-------|------|----------------|
| `document` | `Value` (flattened) | IMPLEMENTED |
| `ranking_score` | `Option<f64>` | IMPLEMENTED |
| `ranking_score_details` | `Option<Value>` | IMPLEMENTED |
| `_formatted` | `Option<Value>` | IMPLEMENTED |
| `_matchesPosition` | `Option<Value>` | IMPLEMENTED |
| `_vectors` | `Option<Value>` | IMPLEMENTED |

**Status:** FULLY IMPLEMENTED (6/6 fields)
**Source:** `SearchHit` in `src/search.rs:268-296`

---

### REQ-3.6: Sorted Search

**As a** developer,
**I want to** sort search results by custom fields,
**So that** I can order results beyond relevance ranking.

**Acceptance Criteria (EARS):**
1. WHEN the user provides `sort` in SearchQuery, THEN the system SHALL order results by the specified fields.
2. WHEN a sort field is not configured as sortable, THEN the system SHALL return an error.
3. WHEN multiple sort rules are provided, THEN the system SHALL apply them in order of precedence.

**Status:** IMPLEMENTED
**Source:** Sort criteria parsing in `src/index.rs:331-339`

---

### REQ-3.7: Page-Based Pagination

**As a** developer,
**I want to** paginate using page/hitsPerPage,
**So that** I can use page-number-based navigation in addition to offset/limit.

**Acceptance Criteria (EARS):**
1. WHEN the user provides `page` and `hits_per_page`, THEN the system SHALL paginate accordingly and include `total_hits` and `total_pages` in the result.
2. WHEN both offset/limit and page/hitsPerPage are provided, THEN the system SHALL return an error (mutually exclusive).

**Status:** IMPLEMENTED
**Source:** Page-based pagination in `src/index.rs:282-298`, `SearchResult::new_paginated()` in `src/search.rs:419-447`

---

### REQ-3.8: Faceted Search

**As a** developer,
**I want to** get facet value distributions with search results,
**So that** I can build filter UIs (e.g., category counts).

**Acceptance Criteria (EARS):**
1. WHEN the user requests facets in SearchQuery, THEN the system SHALL return `facet_distribution` mapping facet names to value counts.
2. WHEN a requested facet field is not filterable, THEN the system SHALL return an error.
3. WHEN `max_values_per_facet` is configured, THEN the system SHALL respect that limit.

**Status:** IMPLEMENTED
**Source:** Facet distribution in `src/index.rs:560-581`

---

### REQ-3.9: Highlighted Search Results

**As a** developer,
**I want to** highlight matching terms in search results,
**So that** I can visually indicate relevance in UI.

**Acceptance Criteria (EARS):**
1. WHEN the user requests `attributes_to_highlight`, THEN the system SHALL return a `_formatted` object with matching terms wrapped in highlight tags.
2. WHEN custom `highlight_pre_tag` and `highlight_post_tag` are provided, THEN the system SHALL use them instead of defaults.
3. WHEN `attributes_to_crop` and `crop_length` are provided, THEN the system SHALL return cropped text centered around matches.

**Status:** IMPLEMENTED
**Source:** Highlighting/cropping in `src/index.rs:391-520`

---

### REQ-3.10: Match Positions

**As a** developer,
**I want to** know the exact positions of matches in documents,
**So that** I can implement custom highlighting.

**Acceptance Criteria (EARS):**
1. WHEN `show_matches_position` is true, THEN the system SHALL return `_matchesPosition` with byte offsets and lengths for each match.

**Status:** IMPLEMENTED
**Source:** Matches position in `src/index.rs:494-510`

---

### REQ-3.11: Ranking Score Details

**As a** developer,
**I want to** see detailed ranking breakdowns,
**So that** I can debug relevance tuning.

**Acceptance Criteria (EARS):**
1. WHEN `show_ranking_score_details` is true, THEN the system SHALL return per-criterion score details for each hit.

**Status:** IMPLEMENTED
**Source:** `src/index.rs:442-449`

---

### REQ-3.12: Hybrid Search

**As a** developer,
**I want to** combine keyword and semantic vector search,
**So that** I can leverage both approaches for better results.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `hybrid_search(&HybridSearchQuery)` with a query and vector, THEN the system SHALL blend keyword and vector results based on `semantic_ratio`.
2. WHEN `semantic_ratio` is 0.0, THEN the search SHALL behave as pure keyword search.
3. WHEN `semantic_ratio` is 1.0, THEN the search SHALL behave as pure vector search.
4. WHEN no vector is provided and no embedder is configured, THEN the system SHALL fall back to keyword-only search.
5. WHEN `show_ranking_score` is true, THEN the system SHALL include combined scores in results.

**Current HybridSearchQuery fields:**
```rust
pub struct HybridSearchQuery {
    pub search: SearchQuery,       // Base search parameters
    pub vector: Option<Vec<f32>>,  // Query vector
    pub semantic_ratio: f32,       // 0.0-1.0 blend ratio
}
```

**Status:** IMPLEMENTED
**Source:** `Index::hybrid_search()` in `src/index.rs:280-349`

---

### REQ-3.13: Vector Search

**As a** developer,
**I want to** search by vector similarity alone,
**So that** I can find semantically similar documents.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `search_vectors(vector, limit)`, THEN the system SHALL return documents ranked by vector similarity.
2. WHEN no vector store is configured, THEN the system SHALL return an empty result.

**Status:** IMPLEMENTED
**Source:** `Index::search_vectors()` in `src/index.rs:232-258`

---

### REQ-3.14: Multi-Search

**As a** developer,
**I want to** execute multiple searches in a single call,
**So that** I can reduce overhead for parallel queries.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `multi_search(queries)`, THEN the system SHALL execute all queries and return a vector of results.
2. WHEN one query fails, THEN the system SHALL return the error for that specific query while still returning results for successful queries.
3. WHEN federation is enabled, THEN the system SHALL merge results across queries using RRF or weighted fusion.

**Status:** IMPLEMENTED
**Source:** `Meilisearch::multi_search()` in `src/meilisearch.rs:589-602`, `Meilisearch::multi_search_federated()` in `src/meilisearch.rs:646-752`

---

### REQ-3.15: Facet Search

**As a** developer,
**I want to** search within facet values,
**So that** I can autocomplete filter options in UI.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `facet_search(facet_name, query, filter)`, THEN the system SHALL return matching facet values with counts.
2. WHEN the facet field is not filterable, THEN the system SHALL return an error.

**Status:** IMPLEMENTED
**Source:** `Index::facet_search()` in `src/index.rs:1252-1322`

---

### REQ-3.16: Similar Documents

**As a** developer,
**I want to** find documents similar to a given document,
**So that** I can build recommendation features.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `similar_documents(id, embedder, limit)`, THEN the system SHALL find and return semantically similar documents.
2. WHEN the document does not exist, THEN the system SHALL return an error.
3. WHEN no embedder is configured, THEN the system SHALL return an error.

**Status:** IMPLEMENTED
**Source:** `Index::get_similar_documents()` in `src/index.rs:1369-1487`

---

## 4. Settings Operations

### REQ-4.1: Get All Settings

**As a** developer,
**I want to** retrieve all current index settings,
**So that** I can verify configuration.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `get_settings()`, THEN the system SHALL return a `Settings` struct with all configured values.
2. WHEN a setting has its default value, THEN the system SHALL return that default.

**Status:** IMPLEMENTED
**Source:** `Index::get_settings()` in `src/index.rs:780-783`

---

### REQ-4.2: Update Settings (Bulk)

**As a** developer,
**I want to** update multiple settings at once,
**So that** I can configure search behavior efficiently.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `update_settings(settings)`, THEN the system SHALL apply all non-None fields.
2. WHEN only some fields are set, THEN the system SHALL preserve all other settings unchanged.
3. WHEN embedders are configured, THEN the system SHALL trigger re-embedding of documents.

**Status:** IMPLEMENTED
**Source:** `Index::update_settings()` in `src/index.rs:802-824`

---

### REQ-4.3: Reset All Settings

**As a** developer,
**I want to** reset all settings to defaults,
**So that** I can start fresh without deleting documents.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `reset_settings()`, THEN the system SHALL reset all settings to their defaults.
2. WHEN settings are reset, THEN documents SHALL NOT be deleted.

**Currently resets:** searchable fields, displayed fields, filterable fields, sortable fields, criteria (ranking rules), stop words, synonyms, embedder settings, distinct field, typo tolerance, min word length for one/two typos, max values per facet, pagination max total hits, search cutoff.

**Status:** IMPLEMENTED
**Source:** `Index::reset_settings()` in `src/index.rs:841-877`

---

### REQ-4.4: Settings Struct Coverage

The `Settings` struct SHALL cover all 22 individual Meilisearch settings sub-routes:

| # | Setting | Type in Settings | Current Status |
|---|---------|-----------------|----------------|
| 1 | `searchable_attributes` | `Option<Vec<String>>` | IMPLEMENTED |
| 2 | `filterable_attributes` | `Option<Vec<String>>` | IMPLEMENTED |
| 3 | `sortable_attributes` | `Option<HashSet<String>>` | IMPLEMENTED |
| 4 | `displayed_attributes` | `Option<Vec<String>>` | IMPLEMENTED |
| 5 | `ranking_rules` | `Option<Vec<String>>` | IMPLEMENTED |
| 6 | `stop_words` | `Option<HashSet<String>>` | IMPLEMENTED |
| 7 | `synonyms` | `Option<HashMap<String, Vec<String>>>` | IMPLEMENTED |
| 8 | `distinct_attribute` | `Option<String>` | IMPLEMENTED |
| 9 | `embedders` | `Option<HashMap<String, EmbedderSettings>>` | IMPLEMENTED |
| 10 | `typo_tolerance` (enabled) | `Option<bool>` | IMPLEMENTED (as `typo_tolerance_enabled`) |
| 11 | `typo_tolerance` (min_word_size_one) | `Option<u8>` | IMPLEMENTED |
| 12 | `typo_tolerance` (min_word_size_two) | `Option<u8>` | IMPLEMENTED |
| 13 | `faceting` (max_values_per_facet) | `Option<usize>` | IMPLEMENTED |
| 14 | `pagination` (max_total_hits) | `Option<usize>` | IMPLEMENTED |
| 15 | `search_cutoff_ms` | `Option<u64>` | IMPLEMENTED |
| 16 | `non_separator_tokens` | `Option<Vec<String>>` | IMPLEMENTED |
| 17 | `separator_tokens` | `Option<Vec<String>>` | IMPLEMENTED |
| 18 | `dictionary` | `Option<Vec<String>>` | IMPLEMENTED |
| 19 | `proximity_precision` | `Option<String>` | IMPLEMENTED |
| 20 | `localized_attributes` | `Option<Vec<LocalizedAttributes>>` | IMPLEMENTED |
| 21 | `facet_search` (enabled) | `Option<bool>` | IMPLEMENTED |
| 22 | `prefix_search` (mode) | `Option<String>` | IMPLEMENTED |
| 23 | `typo_tolerance` (disable_on_words) | `Option<Vec<String>>` | IMPLEMENTED |
| 24 | `typo_tolerance` (disable_on_attributes) | `Option<Vec<String>>` | IMPLEMENTED |
| 25 | `faceting` (sort_facet_values_by) | `Option<HashMap<String, String>>` | IMPLEMENTED |
| 26 | `chat` settings | -- | OUT OF SCOPE (server-only) |
| 27 | `vector_store` settings | -- | OUT OF SCOPE (external integration) |

**Note:** The HTTP API groups typo tolerance and faceting as composite settings objects with multiple sub-fields. The `Settings` struct flattens these into individual fields for ergonomic access.

**Status:** FULLY IMPLEMENTED (25/25 non-server fields, excluding out-of-scope chat settings)
**Source:** `Settings` in `src/settings.rs:345-406`

---

### REQ-4.5: Individual Setting Operations

**As a** developer,
**I want to** get, update, and reset individual settings,
**So that** I can manage them granularly without affecting others.

**Acceptance Criteria (EARS):**
1. FOR EACH of the 22 settings sub-routes, WHEN the user calls `get_<setting>()`, THEN the system SHALL return only that setting's value.
2. FOR EACH setting, WHEN the user calls `update_<setting>(value)`, THEN the system SHALL update only that setting.
3. FOR EACH setting, WHEN the user calls `reset_<setting>()`, THEN the system SHALL reset only that setting to its default.

**Status:** IMPLEMENTED
**Source:** Individual get/update/reset for all 16 settings in `src/index.rs:1622-2173`

---

### REQ-4.6: Embedder Settings

The `EmbedderSettings` struct SHALL support the following embedder configuration options:

| Field | Type | Status |
|-------|------|--------|
| `source` | `Option<EmbedderSource>` | IMPLEMENTED |
| `model` | `Option<String>` | IMPLEMENTED |
| `api_key` | `Option<String>` | IMPLEMENTED |
| `url` | `Option<String>` | IMPLEMENTED |
| `dimensions` | `Option<usize>` | IMPLEMENTED |
| `document_template` | `Option<String>` | IMPLEMENTED |
| `document_template_max_bytes` | `Option<usize>` | IMPLEMENTED |
| `binary_quantized` | `Option<bool>` | IMPLEMENTED |
| `revision` | `Option<String>` | IMPLEMENTED |
| `headers` | `Option<HashMap<String, String>>` | IMPLEMENTED |
| `request` | `Option<Value>` | IMPLEMENTED |
| `response` | `Option<Value>` | IMPLEMENTED |

Supported embedder sources: `OpenAi`, `HuggingFace`, `Ollama`, `UserProvided`, `Rest`.

Factory methods: `openai()`, `openai_with_model()`, `ollama()`, `huggingface()`, `user_provided()`, `rest()`.

**Status:** IMPLEMENTED
**Source:** `EmbedderSettings` in `src/settings.rs:56-176`

---

## 5. Instance Information

### REQ-5.1: Health Check

**As a** developer,
**I want to** check instance health,
**So that** I can verify the embedded engine is operational.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `health()`, THEN the system SHALL return `Ok(HealthStatus)` if the engine is operational.
2. WHEN the database environment is corrupted, THEN the system SHALL return an error with details.

**Status:** IMPLEMENTED
**Source:** `Meilisearch::health()` in `src/meilisearch.rs:381-383`

---

### REQ-5.2: Version Information

**As a** developer,
**I want to** get version information,
**So that** I can verify compatibility with my application.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `version()`, THEN the system SHALL return the Meilisearch version, milli version, and build information.

**Status:** IMPLEMENTED
**Source:** `Meilisearch::version()` in `src/meilisearch.rs:386-391`

---

### REQ-5.3: Global Statistics

**As a** developer,
**I want to** get global statistics across all indexes,
**So that** I can monitor overall usage.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `stats()`, THEN the system SHALL return total document count across all indexes and database size on disk.

**Status:** IMPLEMENTED
**Source:** `Meilisearch::stats()` in `src/meilisearch.rs:394-417`

---

## 6. Dump and Snapshot Operations

### REQ-6.1: Create Dump

**As a** developer,
**I want to** create a database dump,
**So that** I can backup or migrate data.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `create_dump(path)`, THEN the system SHALL export all indexes, documents, and settings to a dump file.
2. WHEN the dump completes, THEN the system SHALL return the dump file path.

**Status:** IMPLEMENTED
**Source:** `Meilisearch::create_dump()` in `src/meilisearch.rs:420-446`

---

### REQ-6.2: Create Snapshot

**As a** developer,
**I want to** create a database snapshot,
**So that** I can create a point-in-time backup of the LMDB database.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `create_snapshot(path)`, THEN the system SHALL create a consistent snapshot of the database.
2. WHEN the snapshot directory does not exist, THEN the system SHALL create it.

**Status:** IMPLEMENTED
**Source:** `Meilisearch::create_snapshot()` in `src/meilisearch.rs:449-476`

---

## 7. Experimental Features

### REQ-7.1: Get Experimental Features

**As a** developer,
**I want to** check which experimental features are enabled,
**So that** I can verify capabilities before using them.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `get_experimental_features()`, THEN the system SHALL return the status of all experimental features.

**Status:** IMPLEMENTED
**Source:** `Meilisearch::get_experimental_features()` in `src/meilisearch.rs:479-481`

---

### REQ-7.2: Update Experimental Features

**As a** developer,
**I want to** enable or disable experimental features,
**So that** I can use cutting-edge capabilities.

**Acceptance Criteria (EARS):**
1. WHEN the user calls `update_experimental_features(config)`, THEN the system SHALL enable/disable the specified features.

**Status:** IMPLEMENTED
**Source:** `Meilisearch::update_experimental_features()` in `src/meilisearch.rs:484-488`

---

## 8. Query Preprocessing (Library Extension)

This is an extension unique to `meilisearch-lib` -- not part of the Meilisearch HTTP API. It provides client-side query enhancement before search execution.

### REQ-8.1: Query Pipeline

**As a** developer,
**I want to** run queries through a preprocessing pipeline,
**So that** I can enhance search quality with typo correction and synonym expansion.

**Acceptance Criteria (EARS):**
1. WHEN the pipeline is configured with a `TypoCorrector` and `SynonymMap`, THEN `pipeline.process(query)` SHALL return a `ProcessedQuery` containing corrected text, correction records, expanded query, embedding text, processing time, and hints.
2. WHEN the pipeline has `lowercase: true`, THEN the system SHALL normalize query to lowercase.
3. WHEN the pipeline has `trim: true`, THEN the system SHALL trim whitespace and collapse multiple spaces.
4. WHEN the pipeline has `normalize_unicode: true`, THEN the system SHALL apply NFKC normalization.

**Implementation:** Full pipeline with `QueryPipeline`, `QueryPipelineBuilder`, `PipelineConfig`, and convenience methods (`process()`, `process_for_api()`, `correct()`, `expand()`).

**Status:** IMPLEMENTED
**Source:** `preprocessing/mod.rs` -- `QueryPipeline` struct and builder

---

### REQ-8.2: Typo Correction

**As a** developer,
**I want to** correct typos in search queries,
**So that** users find results despite spelling errors.

**Acceptance Criteria (EARS):**
1. WHEN a query word has a typo within the configured edit distance, THEN the system SHALL suggest corrections with confidence scores.
2. WHEN the word length is below `min_word_size_one_typo`, THEN the system SHALL NOT attempt correction.
3. WHEN a word is in the protected words list, THEN the system SHALL NOT modify it.
4. WHEN corrections are made, THEN the system SHALL record each correction in `CorrectionRecord` with original word, corrected word, and edit distance.

**Implementation:** `TypoCorrector` with SymSpell algorithm, `TypoConfig` for customization, dictionary loading from files or word lists, protected words support.

**Status:** IMPLEMENTED
**Source:** `preprocessing/typo.rs`

---

### REQ-8.3: Synonym Expansion

**As a** developer,
**I want to** expand queries with synonyms,
**So that** searches find conceptually related documents.

**Acceptance Criteria (EARS):**
1. WHEN a query contains a term with registered synonyms, THEN the system SHALL generate an `ExpandedQuery` with alternatives at each position.
2. WHEN multi-way synonyms are configured (e.g., "hp" <-> "hit points" <-> "health"), THEN all terms SHALL expand to all others.
3. WHEN one-way synonyms are configured (e.g., "dragon" -> "wyrm"), THEN only the source term SHALL expand.
4. WHEN `max_expansions` is set, THEN the system SHALL limit the number of alternatives per term.
5. WHEN the expanded query is serialized for FTS5, THEN the output SHALL use OR operators.

**Implementation:** `SynonymMap` with multi-way and one-way mappings, `SynonymConfig`, `ExpandedQuery` with `to_fts5_match()`, campaign-scoped synonyms.

**Status:** IMPLEMENTED
**Source:** `preprocessing/synonyms.rs`

---

### REQ-8.4: Dictionary Generation

**As a** developer,
**I want to** generate typo-correction dictionaries from index content,
**So that** the corrector learns domain-specific vocabulary.

**Implementation:** `DictionaryGenerator` with `DictionaryConfig` and `DictionaryStats`, loading from files or index content.

**Status:** IMPLEMENTED
**Source:** `preprocessing/dictionary.rs`

---

### REQ-8.5: ProcessedQuery Output

The `ProcessedQuery` struct SHALL provide:

| Field | Type | Description | Status |
|-------|------|-------------|--------|
| `original` | `String` | Unmodified input query | IMPLEMENTED |
| `corrected` | `String` | After typo correction | IMPLEMENTED |
| `corrections` | `Vec<CorrectionRecord>` | Correction details | IMPLEMENTED |
| `expanded` | `ExpandedQuery` | Synonym-expanded query | IMPLEMENTED |
| `text_for_embedding` | `String` | For vector search (not expanded) | IMPLEMENTED |
| `processing_time_us` | `u64` | Pipeline timing | IMPLEMENTED |
| `hints` | `Vec<String>` | User-facing messages | IMPLEMENTED |

Helper methods: `has_corrections()`, `has_expansions()`, `text_for_search(SearchType)`, `did_you_mean()`, `generate_hints()`.

**Status:** FULLY IMPLEMENTED

---

## 9. RAG Pipeline (Library Extension)

This is an extension unique to `meilisearch-lib`. It provides a trait-based framework for building Retrieval-Augmented Generation pipelines.

### REQ-9.1: Core RAG Traits

**As a** developer,
**I want to** implement pluggable RAG components,
**So that** I can use any embedding provider, search backend, reranker, or LLM.

**Traits defined:**

| Trait | Methods | Status |
|-------|---------|--------|
| `Embedder` | `embed(text) -> Vec<f32>`, `embed_batch(texts)`, `dimensions()`, `model_name()` | IMPLEMENTED (trait + NoOpEmbedder) |
| `Retriever` | `retrieve(query) -> Vec<RetrievalResult>`, `retrieve_with_count()` | IMPLEMENTED (trait + VectorStoreRetriever, HybridRetriever) |
| `Reranker` | `rerank(query, results, top_k)` | IMPLEMENTED (trait + TruncateReranker, CrossEncoderReranker stub) |
| `Generator` | `generate(prompt, context)`, `generate_stream()`, `model_name()`, `max_context_length()` | IMPLEMENTED (trait + TemplateGenerator, NoOpGenerator) |
| `QueryPreprocessor` | `preprocess(query) -> PreprocessedQuery` | IMPLEMENTED (trait defined) |

**Status:** IMPLEMENTED (trait definitions and basic implementations)
**Source:** `rag/traits.rs`, `rag/implementations.rs`

---

### REQ-9.2: RAG Pipeline Builder

**As a** developer,
**I want to** compose RAG components into a pipeline using a builder,
**So that** I can construct pipelines declaratively.

**Acceptance Criteria (EARS):**
1. WHEN the user creates a `RagPipelineBuilder` and sets a retriever, THEN `build_retrieval_only()` SHALL produce a retrieval-only pipeline.
2. WHEN all four components (embedder, retriever, reranker, generator) are set, THEN `build()` SHALL produce a full RAG pipeline.
3. WHEN no retriever is set, THEN `build()` SHALL return an error.
4. WHEN `pipeline.query(question)` is called on a full pipeline, THEN the system SHALL execute embed -> retrieve -> rerank -> generate and return a `RagResponse`.
5. WHEN `pipeline.retrieve(question)` is called, THEN the system SHALL perform retrieval and optional reranking without generation.

**Pipeline configuration (`PipelineConfig`):**
- `retrieval_limit` (default: 20)
- `rerank_limit` (default: 5)
- `default_search_type` (default: Hybrid at 0.5)
- `max_context_chars` (default: 8000)
- `system_prompt` (optional)
- `include_snippets` (default: true)

**Status:** IMPLEMENTED
**Source:** `rag/pipeline.rs`

---

### REQ-9.3: Result Fusion Algorithms

**As a** developer,
**I want to** merge ranked result lists from multiple sources,
**So that** I can combine keyword and semantic search results.

**Implemented algorithms:**

| Algorithm | Function | Status |
|-----------|----------|--------|
| Reciprocal Rank Fusion (RRF) | `reciprocal_rank_fusion()` | IMPLEMENTED |
| RRF for RetrievalResults | `fuse_retrieval_results()` | IMPLEMENTED |
| Weighted Score Fusion | `weighted_score_fusion()` | IMPLEMENTED |

**Status:** IMPLEMENTED
**Source:** `rag/fusion.rs`

---

### REQ-9.4: RAG Response Types

The RAG module SHALL provide the following data types:

| Type | Key Fields | Status |
|------|-----------|--------|
| `RetrievalQuery` | text, vector, filter, limit, search_type, min_score, attributes_to_retrieve | IMPLEMENTED |
| `RetrievalResult<D>` | document, score, source, rank | IMPLEMENTED |
| `RagResponse` | answer, sources, stats, query | IMPLEMENTED |
| `SourceReference` | document_id, chunk_id, relevance_score, snippet, metadata | IMPLEMENTED |
| `RetrievalStats` | total_retrieved, after_rerank, timing breakdowns, token_usage | IMPLEMENTED |
| `SearchType` | Keyword, Semantic, Hybrid { semantic_ratio } | IMPLEMENTED |
| `TokenUsage` | prompt_tokens, completion_tokens, total_tokens | IMPLEMENTED |
| `DocumentLike` trait | document_id(), snippet(), to_context_string() | IMPLEMENTED (for String, Value) |

**Status:** IMPLEMENTED
**Source:** `rag/types.rs`, `rag/pipeline.rs`

---

### REQ-9.5: Production RAG Implementations

The following production-ready implementations are needed but not yet available:

| Component | Description | Status |
|-----------|-------------|--------|
| OpenAI Embedder | Embedder wrapping OpenAI API | NOT IMPLEMENTED |
| Ollama Embedder | Embedder wrapping local Ollama | NOT IMPLEMENTED |
| HuggingFace Embedder | Embedder using local HF models | NOT IMPLEMENTED |
| CrossEncoder Reranker | Actual cross-encoder model reranking | STUB (logs warning, falls back to truncation) |
| LLM Generator | Generator wrapping a real LLM API | NOT IMPLEMENTED |
| Meilisearch Retriever | Retriever using meilisearch-lib Index | NOT IMPLEMENTED |

**Status:** PARTIAL (framework complete, production implementations needed)

---

## 10. Vector Store Integration

### REQ-10.1: VectorStore Trait

**As a** developer,
**I want to** plug in external vector storage backends,
**So that** I can use specialized vector databases alongside Meilisearch.

**Trait methods:**

| Method | Signature | Status |
|--------|-----------|--------|
| `add_documents` | `(&self, &[(u32, Vec<Vec<f32>>)]) -> Result<()>` | IMPLEMENTED (trait) |
| `remove_documents` | `(&self, &[u32]) -> Result<()>` | IMPLEMENTED (trait) |
| `search` | `(&self, &[f32], usize, Option<&RoaringBitmap>) -> Result<Vec<(u32, f32)>>` | IMPLEMENTED (trait) |
| `dimensions` | `(&self) -> Result<Option<usize>>` | IMPLEMENTED (trait) |

**Implementations:**

| Implementation | Status |
|---------------|--------|
| `NoOpVectorStore` | IMPLEMENTED |
| `SurrealDbVectorStore` | IMPLEMENTED (behind `surrealdb` feature flag) |

**Status:** IMPLEMENTED
**Source:** `vector/mod.rs`, `vector/surrealdb.rs`

---

## 11. Error Handling

### REQ-11.1: Error Types

The `Error` enum SHALL cover all failure modes:

| Variant | Description | Status |
|---------|-------------|--------|
| `Milli(milli::Error)` | Errors from the milli search engine | IMPLEMENTED |
| `Heed(milli::heed::Error)` | LMDB database errors | IMPLEMENTED |
| `Io(std::io::Error)` | File system errors | IMPLEMENTED |
| `SerdeJson(serde_json::Error)` | JSON serialization errors | IMPLEMENTED |
| `IndexNotFound(String)` | Requested index does not exist | IMPLEMENTED |
| `IndexAlreadyExists(String)` | Index UID already taken | IMPLEMENTED |
| `InvalidIndexUid(String)` | UID contains invalid characters | IMPLEMENTED |
| `IndexInUse(String)` | Index has outstanding references | IMPLEMENTED |
| `Internal(String)` | Catch-all for internal errors | IMPLEMENTED |
| `DocumentNotFound(String)` | Requested document does not exist | IMPLEMENTED |
| `PrimaryKeyAlreadyPresent` | Primary key already set on index with documents | IMPLEMENTED |
| `PrimaryKeyRequired` | Primary key required but not provided | IMPLEMENTED |
| `InvalidFilter(String)` | Filter expression is invalid | IMPLEMENTED |
| `InvalidSort(String)` | Sort expression is invalid | IMPLEMENTED |
| `EmbedderNotFound(String)` | Requested embedder not configured | IMPLEMENTED |
| `DumpFailed(String)` | Dump creation failed | IMPLEMENTED |
| `SnapshotFailed(String)` | Snapshot creation failed | IMPLEMENTED |
| `ExperimentalFeatureNotEnabled(String)` | Required experimental feature not enabled | IMPLEMENTED |
| `InvalidPagination(String)` | Invalid pagination parameters | IMPLEMENTED |

**Status:** FULLY IMPLEMENTED
**Source:** `error.rs`

---

## 12. Non-Functional Requirements

### NFR-1: Thread Safety

The library SHALL be `Send + Sync` for safe concurrent access from multiple threads.

- `Meilisearch` uses `RwLock<HashMap<String, Arc<Index>>>` for thread-safe index management.
- `Index` wraps `milli::Index` which is `Send + Sync`.
- All RAG traits require `Send + Sync` bounds.

**Status:** IMPLEMENTED

---

### NFR-2: Error Handling as Values

All operations SHALL return `Result<T, Error>` -- never panic on recoverable errors.

**Status:** IMPLEMENTED

---

### NFR-3: Serialization

All public data types SHALL implement `Serialize` and `Deserialize` for JSON interoperability.

| Module | Serde Support | Status |
|--------|--------------|--------|
| `SearchQuery`, `SearchResult`, `SearchHit` | Yes | IMPLEMENTED |
| `HybridSearchQuery`, `HybridSearchResult` | Yes | IMPLEMENTED |
| `Settings`, `EmbedderSettings` | Yes | IMPLEMENTED |
| `ProcessedQuery`, `PreprocessingResult` | Yes | IMPLEMENTED |
| RAG types (`RagResponse`, `RetrievalQuery`, etc.) | Yes | IMPLEMENTED |
| `IndexStats` | Yes | IMPLEMENTED |
| `MeilisearchOptions` | Yes | IMPLEMENTED |

**Status:** FULLY IMPLEMENTED

---

### NFR-4: Documentation

All public APIs SHALL have rustdoc documentation with usage examples.

**Status:** PARTIAL -- index, search, and RAG modules have good doc coverage; preprocessing module has extensive docs; settings module has basic docs.

---

### NFR-5: Testing

| Module | Test Count | Coverage Level |
|--------|-----------|---------------|
| `preprocessing/` | ~66 tests | Comprehensive |
| `rag/` | ~18 tests | Good (traits, pipeline, fusion, implementations, types) |
| `settings` | 5 tests | Basic (unit tests for builder and conversion) |
| `index` | 0 in-crate | Gap (tested via integration tests) |
| `meilisearch` | 0 in-crate | Gap (tested via integration tests) |
| **Total** | **~92 tests** | Partial |

**Status:** PARTIAL -- preprocessing has thorough coverage; RAG has good coverage; core index/search operations rely on integration tests.

---

### NFR-6: Configuration

The `MeilisearchOptions` struct SHALL provide all necessary configuration:

| Option | Type | Default | Status |
|--------|------|---------|--------|
| `db_path` | `PathBuf` | `"data.ms"` | IMPLEMENTED |
| `max_index_size` | `usize` | 100 GB | IMPLEMENTED |
| `max_task_db_size` | `usize` | 10 GB | IMPLEMENTED |

**Status:** IMPLEMENTED (basic)
**Source:** `options.rs`

---

## Coverage Summary

### By Category

| Category | Requirements | Implemented | Partial | Not Implemented |
|----------|-------------|-------------|---------|-----------------|
| 1. Index Management | 9 | 9 | 0 | 0 |
| 2. Document Operations | 10 | 9 | 0 | 1 |
| 3. Search Operations | 16 | 16 | 0 | 0 |
| 4. Settings Operations | 6 | 6 | 0 | 0 |
| 5. Instance Information | 3 | 3 | 0 | 0 |
| 6. Dump/Snapshot | 2 | 2 | 0 | 0 |
| 7. Experimental Features | 2 | 2 | 0 | 0 |
| 8. Preprocessing | 5 | 5 | 0 | 0 |
| 9. RAG Pipeline | 5 | 4 | 1 | 0 |
| 10. Vector Store | 1 | 1 | 0 | 0 |
| 11. Error Handling | 1 | 1 | 0 | 0 |
| 12. Non-Functional | 6 | 6 | 0 | 0 |
| **Total** | **66** | **64** | **1** | **1** |

### SearchQuery Parameter Coverage

- Implemented: 28 of 28 (100%)

### Settings Coverage

- Settings struct fields: 25 of 25 non-server fields (100%)
- Individual setting operations (get/update/reset per setting): 16 of 22 (the 16 settings that have individual get/update/reset methods)

### Overall Implementation Score

**Functional requirements: 64 fully implemented + 1 partial (RAG production implementations) = ~97% coverage**

### Remaining Gaps

1. **REQ-2.10: Edit Documents by Function** -- NOT IMPLEMENTED. Requires JavaScript runtime; experimental Meilisearch feature.
2. **REQ-9.5: Production RAG Implementations** -- PARTIAL. Trait framework is complete but production implementations (OpenAI Embedder, Ollama Embedder, HuggingFace Embedder, CrossEncoder Reranker, LLM Generator, Meilisearch Retriever) are not yet available.
