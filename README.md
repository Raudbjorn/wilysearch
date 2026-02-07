# wilysearch

An embedded, HTTP-less Meilisearch engine for Rust. Wraps the [milli](https://github.com/meilisearch/milli) indexing engine directly, giving you full-text search, filtering, sorting, faceting, and hybrid vector search without running a server.

## Why?

Meilisearch is excellent, but the standard deployment requires running an HTTP server and communicating over the network. **wilysearch** strips that away:

- **No HTTP layer** -- operations execute synchronously, in-process
- **No task queue** -- mutations complete immediately and return a synthetic `TaskInfo` with `status: Succeeded`
- **Trait-based API** -- 10 domain traits with 107 methods covering the full Meilisearch SDK surface
- **Composable** -- implement only the traits you need, or use the `MeilisearchApi` super-trait for everything
- **Embeddable** -- LMDB-backed storage lives wherever you point it; great for desktop apps, CLI tools, and testing

## Quick Start

```rust
use wilysearch::core::MeilisearchOptions;
use wilysearch::engine::Engine;
use wilysearch::traits::*;
use wilysearch::types::*;
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create an engine with a database directory
    let options = MeilisearchOptions {
        db_path: "/tmp/my-search-db".into(),
        ..Default::default()
    };
    let engine = Engine::new(options)?;

    // Create an index
    engine.create_index(&CreateIndexRequest {
        uid: "movies".to_string(),
        primary_key: Some("id".to_string()),
    })?;

    // Add documents
    let docs = vec![
        json!({ "id": 1, "title": "The Dark Knight", "year": 2008 }),
        json!({ "id": 2, "title": "Inception", "year": 2010 }),
        json!({ "id": 3, "title": "Interstellar", "year": 2014 }),
    ];
    engine.add_or_replace_documents("movies", &docs, &AddDocumentsQuery::default())?;

    // Search
    let results = engine.search("movies", &SearchRequest {
        q: Some("dark knight".to_string()),
        ..Default::default()
    })?;

    for hit in &results.hits {
        println!("{}", hit["title"]);
    }
    Ok(())
}
```

## Architecture

```
wilysearch (public API)
├── engine::Engine          -- single struct implementing all traits
├── traits                  -- 10 domain traits + MeilisearchApi composite
├── types                   -- 48 API-surface structs, 2 enums (camelCase JSON)
└── core                    -- internal milli/LMDB wrapper
    ├── meilisearch.rs      -- Meilisearch facade (index lifecycle, LMDB env)
    ├── index.rs            -- Index operations (2,299 LOC)
    ├── search.rs           -- Search execution (787 LOC)
    ├── settings.rs         -- Settings conversion (1,130 LOC)
    ├── preprocessing/      -- Query pipeline (SymSpell typo + synonym expansion)
    ├── rag/                -- RAG pipeline (Embedder, Retriever, Reranker, Generator)
    └── vector/             -- VectorStore trait + SurrealDB backend
```

**Total: ~15,300 lines of Rust across 30 source files.**

### Trait API

The public surface is organized into 10 domain traits:

| Trait | Methods | Purpose |
|-------|---------|---------|
| `Documents` | 9 | CRUD, batch delete, filter delete |
| `Search` | 4 | Keyword search, similar, multi-search, facet search |
| `Indexes` | 6 | Create, get, list, delete, swap, update |
| `Tasks` | 4 | Get, list, cancel, delete tasks |
| `Batches` | 2 | Get, list batches |
| `SettingsApi` | 63 | Bulk + 20 individual settings (get/update/reset each) |
| `Keys` | 5 | API key management |
| `Webhooks` | 5 | Webhook CRUD |
| `System` | 7 | Health, version, stats, dumps, snapshots, export |
| `ExperimentalFeaturesApi` | 2 | Get/update experimental features |
| **`MeilisearchApi`** | **107** | **Composite super-trait (auto-implemented)** |

All trait methods are synchronous and return `Result<T>` where `Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>`.

### Engine

`Engine` is the single implementation struct. It wraps `core::Meilisearch` and converts between the public `types::*` structs and the internal milli types:

```rust
pub struct Engine {
    inner: core::Meilisearch,    // LMDB-backed milli instance
    task_counter: AtomicU64,      // synthetic task UID generator
    dump_dir: PathBuf,
    snapshot_dir: PathBuf,
}
```

### Type System

All 48 public types in `types.rs` serialize to the same JSON shape as the Meilisearch HTTP API:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub q: Option<String>,
    pub filter: Option<Value>,
    pub sort: Option<Vec<String>>,
    pub facets: Option<Vec<String>>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub page: Option<usize>,
    pub hits_per_page: Option<usize>,
    // ... 20+ fields matching the HTTP API
}
```

## Features

### Core Search
- Full-text search with BM25 ranking
- Typo tolerance (configurable per word length)
- Filters (`year > 2000`, `genres = "Action"`)
- Sorting (`year:desc`, `rating:asc`)
- Faceted search with distribution counts
- Highlighting and cropping
- Distinct attribute deduplication
- Offset/limit and page-based pagination

### Query Preprocessing
- **SymSpell typo correction** -- O(1) lookups for spelling correction
- **Synonym expansion** -- configurable synonym maps (TOML/JSON)
- **Query pipeline** -- composable `TypoCorrector` -> `SynonymMap` -> search

### RAG Pipeline
- `Embedder`, `Retriever`, `Reranker`, `Generator` traits
- Reciprocal Rank Fusion for hybrid (keyword + vector) results
- Pluggable implementations

### Settings
- 20 individual settings with get/update/reset for each
- Includes: ranking rules, searchable/filterable/sortable/displayed attributes, stop words, synonyms, typo tolerance, pagination, faceting, dictionary, separator tokens, proximity precision, embedders, localized attributes, and more

## Feature Flags

| Flag | Default | Description |
|------|---------|-------------|
| `surrealdb` | off | SurrealDB vector store backend (`kv-mem` + `kv-rocksdb`) |

```toml
[dependencies]
wilysearch = { path = ".", features = ["surrealdb"] }
```

## Dependencies

wilysearch pins against **Meilisearch v1.35.0** for its core crates:

| Category | Crates |
|----------|--------|
| **Meilisearch Core** | `milli`, `meilisearch-types`, `file-store`, `http-client` (git tag v1.35.0) |
| **Serialization** | `serde`, `serde_json`, `indexmap`, `toml` |
| **ML / Embeddings** | `candle-core`, `candle-nn`, `candle-transformers` (pinned `=0.9.1`) |
| **Query Processing** | `symspell`, `strsim` |
| **Data Structures** | `uuid`, `time`, `roaring`, `fst`, `bumpalo` |
| **Concurrency** | `crossbeam-channel`, `rayon` |
| **Error Handling** | `thiserror`, `anyhow`, `log`, `tracing` |
| **Optional** | `surrealdb`, `tokio` (behind `surrealdb` feature) |

> **Note:** `candle-core` is pinned to `=0.9.1` because 0.9.2 pulls in `zip 7.x` -> `typed-path 0.12`, which introduces an ambiguous `AsRef` impl that breaks milli's compilation. This matches upstream's Cargo.lock for Meilisearch v1.35.0.

## Testing

42 integration tests covering the trait API surface:

```bash
cargo test
```

| Test File | Tests | Coverage |
|-----------|-------|----------|
| `index_tests.rs` | 10 | Create, get, delete, list, pagination, stats, health, version |
| `document_tests.rs` | 9 | CRUD, batch delete, filter delete, pagination |
| `search_tests.rs` | 15 | Keywords, filters, sort, facets, highlighting, pagination, crop, distinct, matching strategy |
| `settings_tests.rs` | 8 | Bulk update/reset, individual per-setting accessors |

Tests use `tempfile::TempDir` for isolated LMDB environments that are automatically cleaned up.

## Examples

Five runnable examples in `examples/`:

```bash
cargo run --example basic_search
cargo run --example hybrid_search
cargo run --example multi_search
cargo run --example preprocessing
cargo run --example settings
```

> **Note:** Examples use the lower-level `core::` API directly. The recommended public API is the trait-based `Engine` + `traits::*` + `types::*` interface shown in Quick Start.

## Project Layout

```
wilysearch/
├── src/
│   ├── lib.rs              -- crate root (4 modules)
│   ├── engine.rs           -- Engine struct (1,415 LOC)
│   ├── traits.rs           -- 10 traits, 107 methods (340 LOC)
│   ├── types.rs            -- 48 structs, 2 enums (669 LOC)
│   └── core/               -- internal milli wrapper
│       ├── mod.rs           -- module root + re-exports
│       ├── meilisearch.rs   -- Meilisearch facade (824 LOC)
│       ├── index.rs         -- index operations (2,299 LOC)
│       ├── search.rs        -- search execution (787 LOC)
│       ├── settings.rs      -- settings conversion (1,130 LOC)
│       ├── preprocessing/   -- typo + synonym pipeline (3,671 LOC)
│       ├── rag/             -- RAG pipeline (1,106 LOC)
│       └── vector/          -- vector store (826 LOC)
├── tests/
│   ├── common/mod.rs        -- TestContext + sample data
│   ├── index_tests.rs
│   ├── document_tests.rs
│   ├── search_tests.rs
│   └── settings_tests.rs
├── examples/
│   ├── basic_search.rs
│   ├── hybrid_search.rs
│   ├── multi_search.rs
│   ├── preprocessing.rs
│   └── settings.rs
└── docs/
    ├── dependency-reduction-analysis.md  -- future: replacing milli
    └── tool-execution-spec.md           -- future: LLM tool calling
```

## Roadmap

See `docs/` for detailed design documents:

- **Dependency Reduction** (`docs/dependency-reduction-analysis.md`) -- analysis of replacing milli with lighter backends (Tantivy, sqlite-vec, SurrealDB) for smaller deployments
- **Tool Execution** (`docs/tool-execution-spec.md`) -- LLM tool/function calling architecture for agent integration

## Rust Edition

This project uses **Rust edition 2024**.

## License

MIT
