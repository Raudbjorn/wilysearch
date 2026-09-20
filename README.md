# wilysearch

An embedded, HTTP-less Meilisearch engine for Rust. Wraps the [milli](https://github.com/meilisearch/milli) indexing engine directly, giving you full-text search, filtering, sorting, faceting, and hybrid vector search without running a server.

**0.2 uses a new storage format and requires rebuilding 0.1 databases.** See the
[capability matrix and upgrade guide](docs/parity-0.2.md) before updating.

## Why?

Meilisearch is excellent, but the standard deployment requires running an HTTP server and communicating over the network. **wilysearch** strips that away:

- **No HTTP server** -- index operations execute synchronously, in-process; optional chat uses async provider clients
- **No task queue** -- mutations complete immediately and return a synthetic `TaskInfo` with `status: Succeeded`
- **Trait-based API** -- synchronous document, index, settings and search traits; local APIs for rules, templates and field metadata
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

## Configuration

wilysearch provides unified configuration via TOML files, environment variables, and programmatic Rust structs. Sources are layered with later sources taking precedence: defaults < TOML file < environment variables < programmatic overrides.

### From a config file

```rust
use wilysearch::engine::Engine;

let engine = Engine::from_config_file("wilysearch.toml")?;
```

### Programmatic

```rust
use wilysearch::config::{WilysearchConfig, EngineConfig};
use wilysearch::engine::Engine;

let config = WilysearchConfig {
    engine: EngineConfig {
        db_path: "/var/lib/wilysearch".into(),
        ..Default::default()
    },
    ..Default::default()
};
let engine = Engine::with_config(config)?;
```

### Minimal TOML example

```toml
[engine]
db_path = "/var/lib/wilysearch"

[preprocessing.typo]
maxEditDistance = 1

[search_defaults]
limit = 50
```

### Environment variable overrides

Any setting can be overridden at deploy time with `WILYSEARCH__<SECTION>__<FIELD>`:

```bash
export WILYSEARCH__ENGINE__DB_PATH=/data/search
export WILYSEARCH__SEARCH_DEFAULTS__LIMIT=100
```

See [docs/configuration.md](docs/configuration.md) for the full configuration reference, including all sections, field types, defaults, validation rules, and the complete environment variable mapping table.

## Architecture

```
wilysearch (public API)
├── engine::Engine          -- single struct implementing all traits
├── traits                  -- 10 domain traits + MeilisearchApi composite
├── types                   -- request/response types and upstream settings
├── ai/                     -- optional chat retrieval and Cohere reranking
└── core                    -- internal milli/LMDB wrapper
    ├── meilisearch/        -- index lifecycle, federation, joins and rules
    ├── index/              -- indexing, search, settings and document operations
    ├── search.rs           -- core query and result types
    ├── settings.rs         -- upstream settings aliases
    ├── preprocessing/      -- Query pipeline (SymSpell typo + synonym expansion)
    ├── rag/                -- RAG pipeline (Embedder, Retriever, Reranker, Generator)
    └── vector/             -- VectorStore trait + SurrealDB backend
```

### Trait API

The public surface is organized into 10 domain traits:

| Trait | Methods | Purpose |
|-------|---------|---------|
| `Documents` | 9 | CRUD, batch delete, filter delete |
| `Search` | 4 | Keyword search, similar, multi-search, facet search |
| `Indexes` | 6 | Create, get, list, delete, swap, update |
| `Tasks` | 4 | Compatibility surface; no persistent task queue |
| `Batches` | 2 | Compatibility surface; no batch scheduler |
| `SettingsApi` | 63 | Bulk + 20 individual settings (get/update/reset each) |
| `Keys` | 5 | Compatibility surface; no server authentication |
| `Webhooks` | 5 | Compatibility surface; no webhook delivery |
| `System` | 7 | Health, version, stats, dumps, snapshots, export |
| `ExperimentalFeaturesApi` | 2 | Get/update experimental features |
| **`MeilisearchApi`** | **107** | **Composite super-trait (auto-implemented)** |

All trait methods are synchronous and return `Result<T, wilysearch::error::Error>`. `Engine` also exposes local methods for rules, template previews, field metadata, rename and Rhai document editing. Chat uses a separate async API.

### Engine

`Engine` wraps `core::Meilisearch` and connects the public `types::*` requests to native milli operations. Use it for cross-index behavior such as foreign keys and dynamic search rules.

### Type System

Request and response fields use Meilisearch's camelCase JSON names. For example:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub q: Option<String>,
    pub filter: Option<Value>,
    pub sort: Option<Vec<String>>,
    pub facets: Option<Vec<String>>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub page: Option<u32>,
    pub hits_per_page: Option<u32>,
    // ... 20+ fields matching the HTTP API
}
```

## Features

### Core Search
- Full-text search with native milli ranking rules
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
- Lossless upstream settings with explicit omission/reset/value semantics
- Includes: ranking rules, searchable/filterable/sortable/displayed attributes, stop words, synonyms, typo tolerance, pagination, faceting, dictionary, separator tokens, proximity precision, embedders, localized attributes, and more

## Feature Flags

| Flag | Default | Description |
|------|---------|-------------|
| `ai` | off | Async chat workspaces/tool loops/streaming and synchronous Cohere personalization |
| `surrealdb` | off | SurrealDB vector store backend (`kv-mem` + `kv-rocksdb`) |

```toml
[dependencies]
wilysearch = { path = ".", features = ["surrealdb"] }
```

## Dependencies

Wilysearch pins its engine and optional chat client to Meilisearch 1.54.0 development,
revision `1380adaacebad8a019d88549a06bae2dd90de249`. Rust 1.98.1 is selected by
`rust-toolchain.toml`. Candle versions follow the upstream dependency graph.

## Testing

The existing integration suite and new parity/provider regressions cover the library and CLI:

```bash
cargo test --workspace
cargo test --workspace --features ai
cargo test --workspace --features surrealdb
cargo test --workspace --all-features
```

Tests use isolated temporary LMDB environments. Provider tests use local HTTP mocks with no real credentials. CI runs all four feature combinations.

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

## Roadmap

See `docs/` for detailed design documents:

- **Dependency Reduction** (`docs/dependency-reduction-analysis.md`) -- analysis of replacing milli with lighter backends (Tantivy, sqlite-vec, SurrealDB) for smaller deployments
- **Tool Execution** (`docs/tool-execution-spec.md`) -- LLM tool/function calling architecture for agent integration

## Rust Edition

This project uses **Rust edition 2024**.

## License

MIT
