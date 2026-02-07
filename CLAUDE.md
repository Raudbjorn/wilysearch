# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

wilysearch is an embedded, HTTP-less Meilisearch engine for Rust. It wraps the milli indexing engine (LMDB-backed) directly, providing full-text search without running a server. All operations are synchronous -- no task queue, no HTTP layer. Rust edition 2024.

## Build & Test Commands

```bash
cargo build                          # build (first build is slow due to milli)
cargo test                           # run all 42 integration tests
cargo test --test index_tests        # run a single test file
cargo test test_basic_search         # run a single test by name
cargo test -- --nocapture            # run tests with stdout visible
cargo check                          # fast type-check without linking
cargo run --example basic_search     # run an example
```

Feature flags:
```bash
cargo build --features surrealdb     # enable SurrealDB vector store backend
```

## Architecture: Two-Layer Type System

The critical design decision to understand: there are **two parallel type systems** that `engine.rs` bridges.

### Public API (what consumers use)
- `src/traits.rs` -- 10 domain traits (107 methods total), all synchronous, returning `Result<T>` where `Result = std::result::Result<T, Box<dyn Error + Send + Sync>>`
- `src/types.rs` -- 48 structs + 2 enums, all `#[serde(rename_all = "camelCase")]`, matching the Meilisearch HTTP API JSON shape
- `src/engine.rs` -- `Engine` struct that implements all 10 traits

### Internal core (milli wrapper)
- `src/core/` -- wraps milli/LMDB directly with its own types (`core::SearchQuery`, `core::Settings`, `core::SearchResult`, etc.)
- `src/core/meilisearch.rs` -- `Meilisearch` facade managing index lifecycle and LMDB environment
- `src/core/index.rs` -- `Index` struct with direct milli operations (2,299 LOC, the largest file)
- `src/core/search.rs` -- search execution types and logic
- `src/core/settings.rs` -- settings conversion between milli and the core layer

### The Bridge: engine.rs

`engine.rs` (1,415 LOC) is the glue. It contains conversion functions:
- `convert_search_request()` -- `types::SearchRequest` -> `core::SearchQuery`
- `convert_search_result()` -- `core::SearchResult` -> `types::SearchResponse`
- `convert_settings_to_lib()` -- `types::Settings` -> `core::Settings`
- `convert_settings_from_lib()` -- `core::Settings` -> `types::Settings`

When adding a new field to the public API, you must update **both** `types.rs` and the relevant conversion function in `engine.rs`.

### Module path convention

Internal core code uses `crate::core::` paths (e.g., `crate::core::Error`, `crate::core::search::MatchingStrategy`). The public API uses `crate::types::*` and `crate::traits::*`. These namespaces must not be confused -- the core has its own `Error`, `Result`, `Settings`, `SearchQuery` etc. that are distinct from the public types.

## Dependency Constraints

- Meilisearch core crates (`milli`, `meilisearch-types`, `file-store`, `http-client`) are pinned to git tag `v1.35.0`
- **candle-core/nn/transformers MUST stay at `=0.9.1`** -- 0.9.2 pulls in `zip 7.x` -> `typed-path 0.12` which introduces an ambiguous `AsRef` impl that breaks milli
- The `surrealdb` feature flag gates `dep:surrealdb` and `dep:tokio`

## Test Structure

Tests live in `tests/` (not `src/`) and use a shared `TestContext`:
```
tests/common/mod.rs     -- TestContext (TempDir + Engine), sample data helpers
tests/index_tests.rs    -- index CRUD, pagination, stats, health, version
tests/document_tests.rs -- document CRUD, batch/filter delete
tests/search_tests.rs   -- keyword search, filters, sort, facets, highlighting, pagination
tests/settings_tests.rs -- bulk settings, individual per-setting get/update/reset
```

Every test creates a fresh `TestContext` with its own temp LMDB directory. Tests use the **public trait API** (`Engine` + `traits::*` + `types::*`), not core internals.

## Subsystems in core/

- `core/preprocessing/` -- SymSpell typo correction + synonym expansion pipeline. Configurable via TOML.
- `core/rag/` -- RAG pipeline with `Embedder`, `Retriever`, `Reranker`, `Generator` traits + RRF fusion.
- `core/vector/` -- `VectorStore` trait with `NoOpVectorStore` default and optional `SurrealDbVectorStore`.
