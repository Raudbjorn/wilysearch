# TTRPS Meilisearch Endpoint Coverage Spec

## Overview

This document maps every Meilisearch HTTP endpoint used by the TTRPS (Tabletop RPG Assistant) desktop application to its equivalent operation in `meilisearch-lib`. It identifies coverage status, behavioral gaps, and integration requirements for migrating TTRPS from HTTP-based access to the embedded library.

**Source analysis:**
- TTRPS codebase: `TTTRPS/src-tauri/src/core/search/client.rs` (~985 lines)
- TTRPS campaign client: `TTTRPS/src-tauri/src/core/campaign/meilisearch_client.rs`
- meilisearch-lib: `crates/meilisearch-lib/src/` (all modules)
- Meilisearch server routes: `crates/meilisearch/src/routes/`

---

## Phase 1: Requirements

### Endpoint-to-API Mapping

Each row maps a TTRPS HTTP call to its `meilisearch-lib` equivalent.

| # | TTRPS Operation | HTTP Endpoint | meilisearch-lib API | Status |
|---|----------------|---------------|---------------------|--------|
| 1 | Health check | `GET /health` | `Meilisearch::health()` | COVERED |
| 2 | List indexes (paginated) | `GET /indexes?limit=100&offset=N` | `Meilisearch::list_indexes_with_pagination(offset, limit)` | COVERED |
| 3 | Index stats | `GET /indexes/{uid}/stats` | `Meilisearch::index_stats(uid)` | COVERED |
| 4 | Create index | `POST /indexes` | `Meilisearch::create_index(uid, primary_key)` | COVERED |
| 5 | Delete index | `DELETE /indexes/{uid}` | `Meilisearch::delete_index(uid)` | COVERED |
| 6 | Add documents (batch) | `POST /indexes/{uid}/documents` | `Index::add_documents(docs, primary_key)` | COVERED |
| 7 | Delete document by ID | `DELETE /indexes/{uid}/documents/{id}` | `Index::delete_document(id)` | COVERED |
| 8 | Delete by filter | search + `DELETE /indexes/{uid}/documents` | `Index::delete_by_filter(filter)` | COVERED |
| 9 | Clear index | `DELETE /indexes/{uid}/documents` | `Index::clear()` | COVERED |
| 10 | Basic search | `POST /indexes/{uid}/search` | `Index::search(&SearchQuery)` | COVERED |
| 11 | Hybrid search | `POST /indexes/{uid}/search` (with `hybrid` body) | `Index::search(&SearchQuery)` with `hybrid` field | COVERED |
| 12 | Federated search | Multiple `POST /search` merged | `Meilisearch::multi_search_federated(queries, federation)` | COVERED |
| 13 | Enable experimental features | `PATCH /experimental-features` | `Meilisearch::update_experimental_features(features)` | COVERED |
| 14 | Update index settings | `PATCH /indexes/{uid}/settings` | `Index::update_settings(&settings)` | COVERED |
| 15 | Update embedders | `PATCH /indexes/{uid}/settings/embedders` | `Index::update_embedders(map)` | COVERED |
| 16 | Get embedder settings | `GET /indexes/{uid}/settings/embedders` | `Index::get_embedders()` | COVERED |
| 17 | Get document by ID | `GET /indexes/{uid}/documents/{id}` | `Index::get_document(id, fields)` | COVERED |
| 18 | Document count | `GET /indexes/{uid}/stats` -> numberOfDocuments | `Index::document_count()` | COVERED |
| 19 | Update documents (partial) | `PUT /indexes/{uid}/documents` | `Index::update_documents(docs, primary_key)` | COVERED |

**Coverage: 19/19 endpoints (100%)**

---

### Behavioral Gap Analysis

While all endpoints have API equivalents, the TTRPS implementation includes behavioral patterns that need equivalent handling in the embedded context:

#### GAP-1: Batch Document Ingestion with Progress

**TTRPS behavior** (`client.rs:397-428`):
- Splits large document sets into batches of 1000 (`MEILISEARCH_BATCH_SIZE`)
- Each batch returns a task ID
- Waits for task completion with a 300s timeout (`TASK_TIMEOUT_LONG_SECS`)
- Emits progress events per batch

**meilisearch-lib behavior:**
- `Index::add_documents()` processes synchronously (no task queue)
- No batching needed for correctness (milli handles arbitrarily large batches)
- No timeout mechanism (operation completes or fails)

**Requirement:**
WHEN migrating TTRPS to embedded mode, THEN batch splitting SHALL be optional (for progress reporting only, not API limits). Progress callbacks SHALL replace task polling.

**Status:** GAP -- TTRPS needs to replace task-polling with synchronous progress callbacks or chunked iteration.

---

#### GAP-2: Retry Logic with Exponential Backoff

**TTRPS behavior** (`meilisearch_client.rs:652-700`):
- Campaign operations retry up to 3 times
- Exponential backoff: 100ms * 2^attempt
- Retries on ConnectionError and TaskTimeout
- Does not retry DocumentNotFound or SerializationError

**meilisearch-lib behavior:**
- Operations are synchronous and deterministic
- No network errors possible (embedded)
- No task timeouts (synchronous execution)

**Requirement:**
WHEN migrating to embedded mode, THEN retry logic SHALL be removed for campaign operations since ConnectionError and TaskTimeout cannot occur in embedded mode. Only application-level errors (concurrent LMDB conflicts) need handling.

**Status:** GAP -- TTRPS retry logic can be simplified but the campaign client abstraction layer must be updated.

---

#### GAP-3: Bearer Token Authentication

**TTRPS behavior** (`client.rs`):
- All HTTP requests include `Authorization: Bearer {api_key}` header
- Master key stored in system keyring

**meilisearch-lib behavior:**
- Embedded library has no authentication boundary
- Operates in a trusted context (same process)

**Requirement:**
WHEN migrating to embedded mode, THEN authentication SHALL be removed from the search client. Access control moves to the Tauri command layer (already present).

**Status:** GAP -- Authentication code should be removed from the embedded integration layer.

---

#### GAP-4: Embedder Configuration via Raw HTTP

**TTRPS behavior** (`client.rs:229-391`):
- Configures embedders via raw HTTP PATCH to `/indexes/{uid}/settings/embedders`
- Uses custom JSON payloads for Ollama REST and Copilot REST embedders
- Gets embedder settings via raw HTTP GET

**meilisearch-lib behavior:**
- `Index::update_embedders()` accepts `HashMap<String, EmbedderSettings>`
- `EmbedderSettings` supports `Rest` source with `url`, `api_key`, `model`, `dimensions`, `headers`, `request`, `response` templates
- Factory method: `EmbedderSettings::rest()`

**Requirement:**
WHEN configuring Ollama or Copilot embedders in embedded mode, THEN the `EmbedderSettings::rest()` factory with custom `request`/`response` JSON templates SHALL produce equivalent behavior to the current raw HTTP PATCH.

EARS: WHEN the TTRPS calls `update_embedders` with an Ollama REST config, THEN the embedder SHALL be configured identically to the HTTP API's `PATCH /settings/embedders` with the same JSON body.

**Status:** COVERED (by `EmbedderSettings::rest()`) but needs verification test.

---

#### GAP-5: Dynamic Per-Document Indexes

**TTRPS behavior** (`client.rs:848-924`):
- Creates per-document indexes dynamically: `<source-slug>-raw` (Phase 1) and `<source-slug>` (Phase 2)
- Each has unique settings (searchable, filterable, sortable attributes)
- `ensure_raw_index()` and `ensure_chunks_index()` are idempotent (create-if-not-exists)

**meilisearch-lib behavior:**
- `Meilisearch::create_index()` returns `IndexAlreadyExists` if the index exists
- No built-in "create-if-not-exists" pattern

**Requirement:**
WHEN ensuring a dynamic index exists in embedded mode, THEN the integration layer SHALL handle `IndexAlreadyExists` gracefully (catch and return existing index). This pattern should be a utility method.

EARS: WHEN `ensure_index(uid, primary_key)` is called and the index already exists, THEN the system SHALL return the existing index handle without error.

**Status:** GAP -- Need a convenience method or pattern for idempotent index creation.

---

#### GAP-6: Federated Search Across Dynamic Indexes

**TTRPS behavior** (`client.rs:623-663`):
- `search_all()` / `federated_search()` searches across core indexes (rules, fiction, documents)
- Plus dynamically created per-document indexes
- Merges results by score
- Returns `FederatedResults { hits, total_hits, processing_time_ms }`

**meilisearch-lib behavior:**
- `Meilisearch::multi_search_federated()` handles cross-index search with merging
- Requires explicit list of target indexes

**Requirement:**
WHEN performing federated search in embedded mode, THEN the integration layer SHALL enumerate all matching indexes (core + dynamic) and pass them to `multi_search_federated()`.

**Status:** COVERED (by multi_search_federated) -- integration layer must handle index enumeration.

---

#### GAP-7: Library Metadata as Index

**TTRPS behavior** (`client.rs:784-820`):
- Uses a dedicated `library_metadata` index to store document metadata
- CRUD operations: save, list, get, delete, rebuild
- Metadata fields: id, name, source_type, file_path, page_count, chunk_count, etc.
- `rebuild_library_metadata()` scans all content indexes to regenerate

**meilisearch-lib behavior:**
- Standard document operations on any index cover all CRUD
- No special "metadata index" concept needed

**Requirement:**
WHEN managing library metadata in embedded mode, THEN standard `add_documents`, `search`, `get_document`, `delete_document` on the `library_metadata` index SHALL suffice.

**Status:** COVERED -- No special API needed.

---

### TTRPS Index Configuration Reference

For completeness, here are all indexes TTRPS creates with their settings:

| Index | Primary Key | Searchable | Filterable | Sortable |
|-------|-------------|------------|------------|----------|
| `ttrpg_rules` | `id` | content, source, metadata | source, source_type, campaign_id, session_id | created_at |
| `ttrpg_fiction` | `id` | content, source, metadata | source, source_type, campaign_id | created_at |
| `ttrpg_chat` | `id` | content, metadata | source, campaign_id, session_id | created_at |
| `ttrpg_documents` | `id` | content, source, metadata | source, source_type | created_at |
| `library_metadata` | `id` | name, file_path, game_system | source_type, game_system, status | ingested_at |
| `ttrpg_campaign_arcs` | `id` | (default) | campaign_id, status | created_at |
| `ttrpg_session_plans` | `id` | (default) | campaign_id, arc_id | session_number |
| `ttrpg_plot_points` | `id` | (default) | campaign_id, arc_id, status | priority |
| `<slug>-raw` | `id` | raw_content, source_slug | source_slug | page_number |
| `<slug>` (chunks) | `id` | 17 attributes | 20+ attributes | page_start, chunk_index |

---

## Phase 2: Design

### Integration Architecture

```
TTRPS (current):
  Tauri Commands → SearchClient (HTTP) → Meilisearch Server → milli

TTRPS (target):
  Tauri Commands → SearchClient (embedded) → meilisearch-lib → milli
```

The migration replaces `SearchClient`'s HTTP implementation with direct `meilisearch-lib` calls while preserving the public interface.

### 2.1 SearchClient Adapter

The existing `SearchClient` struct in TTRPS uses `reqwest::Client` for HTTP calls. The embedded variant replaces it with `Arc<Meilisearch>`:

```rust
// Current (HTTP):
pub struct SearchClient {
    host: String,
    api_key: Option<String>,
    client: reqwest::Client,
}

// Target (Embedded):
pub struct SearchClient {
    meili: Arc<Meilisearch>,
}
```

All public methods on `SearchClient` retain their signatures. Internal implementation changes from HTTP to embedded calls.

### 2.2 Idempotent Index Creation

New utility for TTRPS's create-if-not-exists pattern:

```rust
impl Meilisearch {
    /// Create an index if it doesn't exist, or return the existing one.
    pub fn ensure_index(
        &self,
        uid: &str,
        primary_key: Option<&str>,
    ) -> Result<Arc<Index>> {
        match self.create_index(uid, primary_key) {
            Ok(index) => Ok(index),
            Err(Error::IndexAlreadyExists(_)) => self.get_index(uid),
            Err(e) => Err(e),
        }
    }
}
```

### 2.3 Progress Reporting for Batch Ingestion

Replace task polling with a callback-based approach:

```rust
/// Progress callback for batch document ingestion.
pub type ProgressCallback = Box<dyn Fn(IngestionProgress) + Send>;

pub struct IngestionProgress {
    pub batch_index: usize,
    pub total_batches: usize,
    pub documents_processed: usize,
    pub total_documents: usize,
}

impl Index {
    /// Add documents in batches with progress reporting.
    pub fn add_documents_with_progress(
        &self,
        documents: Vec<Value>,
        primary_key: Option<&str>,
        batch_size: usize,
        on_progress: Option<ProgressCallback>,
    ) -> Result<u64>;
}
```

### 2.4 Campaign Client Simplification

The `MeilisearchCampaignClient` in TTRPS wraps `SearchClient` with retry logic. In embedded mode:

- Remove retry logic (no network errors)
- Remove timeout handling (synchronous execution)
- Keep the domain abstraction (campaign-specific methods)
- Map directly to `Index` operations

---

## Phase 3: Tasks

### Current Status Audit

Based on the `lib.rs` exports and source analysis, the following items from `sdk-coverage-tasks.md` are NOW COMPLETE:

| Phase | Task | Status |
|-------|------|--------|
| 1.1 | Sort, distinct, matchingStrategy, rankingScoreThreshold | DONE |
| 1.2 | Page-based pagination | DONE |
| 1.3 | Facets support | DONE |
| 1.4 | Highlighting and cropping | DONE |
| 1.5 | Match positions and ranking score details | DONE |
| 1.6 | Remaining SearchQuery fields | DONE |
| 1.7 | Hybrid search unification | DONE |
| 2.1 | update_documents (partial) | DONE |
| 2.2 | Field filtering on get_document/get_documents | DONE |
| 2.3 | Missing Settings fields | DONE |
| 2.4 | Individual settings batch 1 | DONE |
| 2.5 | Individual settings batch 2 | DONE |
| 3.1 | Facet search | DONE |
| 3.2 | Multi-search (non-federated) | DONE |
| 3.3 | Federated multi-search | DONE |
| 3.4 | Similar documents | DONE |
| 4.1 | update_primary_key | DONE |
| 4.2 | swap_indexes | DONE |
| 4.3 | list_indexes_with_pagination + IndexInfo | DONE |
| 5.1 | Health check and version info | DONE |
| 5.2 | Global stats | DONE |
| 5.3 | Dump and snapshot | DONE |
| 5.4 | Experimental features | DONE |
| 6.1 | Extended error types | DONE |

**All 24 implementation tasks from the original spec are COMPLETE.**

---

### Remaining Tasks (TTRPS Integration)

The remaining work is TTRPS-specific integration, not core library gaps:

#### TASK-1: Add `ensure_index()` convenience method

**Effort:** 1h | **Deps:** None | **Req:** GAP-5

**Files:**
- `crates/meilisearch-lib/src/meilisearch.rs`

**Implementation:**
- Add `ensure_index(uid, primary_key) -> Result<Arc<Index>>` that handles `IndexAlreadyExists`
- Unit test

**Acceptance criteria:**
- WHEN called for a new index, THEN creates and returns the index
- WHEN called for an existing index, THEN returns the existing index without error
- WHEN the UID is invalid, THEN returns `InvalidIndexUid` error

---

#### TASK-2: Add `add_documents_with_progress()` method

**Effort:** 2h | **Deps:** None | **Req:** GAP-1

**Files:**
- `crates/meilisearch-lib/src/index.rs`

**Implementation:**
- Accept a `batch_size` and optional progress callback
- Split documents into chunks of `batch_size`
- Call `add_documents()` for each chunk
- Invoke callback after each batch
- Return total documents indexed

**Acceptance criteria:**
- WHEN called with 2500 documents and batch_size=1000, THEN processes 3 batches and invokes callback 3 times
- WHEN callback is None, THEN processes all documents normally
- WHEN a batch fails, THEN returns error with partial progress info

---

#### TASK-3: Verify Ollama/REST embedder configuration parity

**Effort:** 2h | **Deps:** None | **Req:** GAP-4

**Files:**
- `crates/meilisearch-lib/tests/settings_tests.rs`

**Implementation:**
- Write integration test that configures an Ollama REST embedder via `EmbedderSettings::rest()`
- Verify the settings round-trip: write then read back
- Compare JSON output to the HTTP API's expected payload shape
- Test Copilot REST configuration similarly

**Acceptance criteria:**
- WHEN an Ollama REST embedder is configured with url, model, dimensions, THEN `get_embedders()` returns matching settings
- WHEN a REST embedder is configured with custom request/response templates, THEN they are preserved

---

#### TASK-4: Update stale SDK coverage documents

**Effort:** 2h | **Deps:** None | **Req:** Documentation accuracy

**Files:**
- `crates/meilisearch-lib/docs/sdk-coverage-requirements.md`
- `crates/meilisearch-lib/docs/sdk-coverage-tasks.md`

**Implementation:**
- Update all "NOT IMPLEMENTED" and "PARTIAL" status markers to reflect current state
- Update the Coverage Summary table
- Mark completed tasks with checkboxes
- Add a "Last verified" date header

**Acceptance criteria:**
- WHEN a developer reads the requirements doc, THEN status markers accurately reflect the implementation
- WHEN a developer reads the tasks doc, THEN completed tasks are clearly marked

---

#### TASK-5: Write TTRPS-pattern integration tests

**Effort:** 4h | **Deps:** TASK-1, TASK-2 | **Req:** GAP-1 through GAP-7

**Files:**
- `crates/meilisearch-lib/tests/ttrps_patterns_tests.rs`

**Tests to write:**

1. **Idempotent index creation** - `ensure_index` called twice returns same index
2. **Batch ingestion with progress** - 2500 docs, batch_size=1000, verify 3 callbacks
3. **Dynamic per-document indexes** - Create `test-slug-raw` and `test-slug` with different settings
4. **Multi-index federated search** - Search across 3+ indexes, verify merged results
5. **Embedder configuration round-trip** - Configure REST embedder, read back, verify
6. **Filter-based deletion** - Add docs with `source_type`, delete by filter, verify count
7. **Library metadata CRUD** - Full lifecycle on a metadata index
8. **Campaign document CRUD** - Add, update, search, delete campaign arc documents
9. **TTRPG search with complex filters** - 5+ filterable attributes, compound filter expression
10. **Hybrid search fallback** - Hybrid search when no embedder configured falls back to keyword
11. **Index stats aggregation** - Create 3 indexes, verify `stats()` totals

**Acceptance criteria:**
- All 11 tests pass with `cargo test`
- Tests use tempdir for isolation (no external dependencies)
- Tests cover all 7 identified gaps

---

#### TASK-6: Document TTRPS migration guide

**Effort:** 2h | **Deps:** TASK-1 through TASK-5 | **Req:** Developer documentation

**Files:**
- `crates/meilisearch-lib/docs/ttrps-migration-guide.md`

**Content:**
- Step-by-step guide for replacing `SearchClient` HTTP calls with embedded calls
- API mapping table (SearchClient method -> meilisearch-lib method)
- Code examples for each migration pattern
- Notes on removing retry logic, authentication, and task polling
- Performance comparison (HTTP overhead eliminated)

**Acceptance criteria:**
- WHEN a developer follows the guide, THEN they can migrate a TTRPS SearchClient method in <30 minutes per method
- All code examples compile

---

### Task Dependency Graph

```
Independent (can start in parallel):
  TASK-1 (ensure_index)
  TASK-2 (add_documents_with_progress)
  TASK-3 (embedder parity test)
  TASK-4 (update docs)

Depends on TASK-1, TASK-2:
  TASK-5 (integration tests)

Depends on TASK-1 through TASK-5:
  TASK-6 (migration guide)
```

### Effort Summary

| Task | Description | Hours |
|------|-------------|-------|
| TASK-1 | ensure_index() | 1h |
| TASK-2 | add_documents_with_progress() | 2h |
| TASK-3 | Embedder config parity test | 2h |
| TASK-4 | Update stale docs | 2h |
| TASK-5 | TTRPS-pattern integration tests | 4h |
| TASK-6 | Migration guide | 2h |
| **Total** | | **13h** |

---

## Appendix A: Meilisearch Server Endpoints NOT Used by TTRPS

For reference, these Meilisearch HTTP endpoints exist but are NOT used by TTRPS and are therefore out of scope for this coverage spec:

| Endpoint | Reason Not Used |
|----------|----------------|
| `POST /tasks/cancel`, `DELETE /tasks`, `GET /tasks/{id}` | Task management is implicit in SDK |
| `GET /batches`, `GET /batches/{id}` | Not used |
| `POST /keys`, `GET /keys`, etc. (5 endpoints) | Authentication handled externally |
| `POST /dumps` | Not used (could be future feature) |
| `POST /snapshots` | Not used |
| `GET /version` | Not used (but available) |
| `POST /multi-search` (direct) | TTRPS does manual multi-index search |
| `POST /indexes/{uid}/facet-search` | Not used |
| `GET/POST /indexes/{uid}/similar` | Not used |
| `POST /indexes/{uid}/compact` | Not used |
| `POST /swap-indexes` | Not used |
| `POST /indexes/{uid}/documents/edit` | Not used |
| `POST /indexes/{uid}/documents/fetch` | Not used |
| `POST /indexes/{uid}/documents/delete-batch` | Not used |
| `GET/PATCH /network` | Enterprise feature |
| `POST /export` | Not used |
| Webhook endpoints (5) | Not used |
| Chat completion endpoints (7) | Handled in separate chat module |
| `POST /logs/stream`, `POST /logs/stderr` | Server-only |
| `GET /metrics` | Server-only |

## Appendix B: TTRPS Index Constants

```rust
pub const INDEX_RULES: &str = "ttrpg_rules";
pub const INDEX_FICTION: &str = "ttrpg_fiction";
pub const INDEX_CHAT: &str = "ttrpg_chat";
pub const INDEX_DOCUMENTS: &str = "ttrpg_documents";
pub const INDEX_LIBRARY_METADATA: &str = "library_metadata";
pub const INDEX_TTRPG: &str = "ttrpg";
pub const INDEX_CAMPAIGN_ARCS: &str = "ttrpg_campaign_arcs";
pub const INDEX_SESSION_PLANS: &str = "ttrpg_session_plans";
pub const INDEX_PLOT_POINTS: &str = "ttrpg_plot_points";

pub const MEILISEARCH_BATCH_SIZE: usize = 1000;
pub const PAGE_SIZE: usize = 100;
pub const TASK_TIMEOUT_SHORT_SECS: u64 = 30;
pub const TASK_TIMEOUT_LONG_SECS: u64 = 300;
```
