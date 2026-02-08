# Unified Configuration Interface -- Design

## Overview

A new `src/config.rs` module introduces `WilysearchConfig` -- a single typed struct that unifies all subsystem configuration. It uses the `figment` crate for layered config loading (defaults < TOML file < environment variables) with provenance-aware error messages. The existing `Engine::new(MeilisearchOptions)` API is preserved; new constructors (`Engine::with_config`, `Engine::from_config_file`) are additive.

---

## Architecture

```
                        Consumer Code
                             |
                    +--------+--------+
                    |                 |
              Programmatic        File-based
            (struct literal)    (TOML + env)
                    |                 |
                    v                 v
             WilysearchConfig   WilysearchConfig::from_file()
                    |                 |
                    +--------+--------+
                             |
                    Engine::with_config(config)
                             |
              +--------------+--------------+
              |              |              |
         MeilisearchOptions  |     ExperimentalFeatures
              |        PreprocessingConfig
         Meilisearch::new()  |
              |         Pipeline/VectorStore
              v              v
           [LMDB]    [Preprocessing + RAG]
```

### Loading Precedence (later wins)

```
1. Rust Default impls          (compiled-in defaults)
2. TOML config file            (optional, specified by consumer)
3. WILYSEARCH__* env vars      (deployment-time overrides)
4. Programmatic overrides      (consumer mutates struct after loading)
```

---

## Decision: Configuration Crate

### Context
wilysearch needs layered config (defaults + file + env vars) for a library crate with 4+ nested subsystems and feature-gated sections.

### Options Considered

| Criterion | `config` crate | `figment` | Hand-rolled | Pure serde |
|-----------|---------------|-----------|-------------|------------|
| Library-author pattern | No | Yes (Provider trait) | N/A | N/A |
| Error provenance | No | Yes | No | No |
| Maintenance per field | Low | Zero (derive) | High | Medium |
| Feature-gated sections | Manual | Natural (serde + cfg) | Manual | Fragile |
| New dependencies | 1 + transitive | 1 (minimal) | 0 | 0 |

### Decision: `figment`

**Rationale:**
1. **Library-first design** -- figment's `Provider` trait lets wilysearch participate in a consumer's own config layering without forcing a specific pattern.
2. **Provenance tracking** -- Error messages like `"invalid type: found string 'abc', expected usize for 'engine.max_index_size' in env var WILYSEARCH__ENGINE__MAX_INDEX_SIZE"` are hard to replicate by hand and invaluable for debugging.
3. **Zero maintenance per field** -- Adding a new config field is just adding a struct field with `#[serde(default)]`. No per-field env var matching code.
4. **Feature-gated sections** -- `#[cfg(feature = "surrealdb")]` on struct fields works naturally; figment ignores unknown sections.
5. **Minimal footprint** -- ~15KB source, deps are only `serde` + `uncased`.

---

## Decision: Replacing CLI Flags

### Context
Meilisearch server uses CLI flags (`--db-path`, `--max-index-size`). wilysearch is a library with no binary -- there is no `main()` to parse flags.

### Decision: Config file + env vars + typed structs

The TOML config file IS the flag equivalent. Instead of:
```bash
meilisearch --db-path ./data --max-index-size 100GiB
```

Consumers either:
```rust
// Programmatic (compile-time)
let config = WilysearchConfig { engine: EngineConfig { db_path: "./data".into(), .. }, .. };
```
```bash
# Environment (deploy-time)
WILYSEARCH__ENGINE__DB_PATH=./data
WILYSEARCH__ENGINE__MAX_INDEX_SIZE=107374182400
```
```toml
# File (declarative)
[engine]
db_path = "./data"
max_index_size = 107_374_182_400
```

This is strictly better than flags for a library:
- Type-safe at compile time (flags are stringly-typed)
- Discoverable via IDE autocomplete
- Composable (multiple files, env layering)
- No runtime argument parser dependency

---

## Decision: Section Naming

### Context
The LMDB/core options could be `[meilisearch]` (matching the upstream project name) or `[engine]` (matching the public `Engine` type).

### Decision: `[engine]`

Rationale: Consumers interact with `Engine`, not `Meilisearch` (the inner type is not public API). Using `[engine]` avoids confusion between "meilisearch the server" and "wilysearch's embedded engine."

---

## Decision: RAG SearchType in TOML

### Context
`PipelineConfig.default_search_type` is `SearchType::Hybrid { semantic_ratio: f32 }` -- an enum with a payload. This doesn't map cleanly to a flat TOML value.

### Decision: Flatten to two fields

```toml
[rag]
default_search_type = "hybrid"   # "keyword" | "semantic" | "hybrid"
semantic_ratio = 0.5             # only used when default_search_type = "hybrid"
```

A custom deserializer reconstructs the `SearchType` enum from these two fields. This keeps the TOML ergonomic.

---

## Components and Interfaces

### `WilysearchConfig` (new: `src/config.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WilysearchConfig {
    pub engine: EngineConfig,
    pub preprocessing: PreprocessingConfig,
    #[cfg(feature = "surrealdb")]
    pub vector_store: Option<VectorStoreConfig>,
    pub rag: RagConfig,
    pub experimental: ExperimentalFeatures,
    pub search_defaults: SearchDefaultsConfig,
}
```

### `EngineConfig` (new: `src/config.rs`)

Wraps `MeilisearchOptions` fields plus new tunables:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineConfig {
    /// Directory where the LMDB database files are stored.
    pub db_path: PathBuf,           // default: "data.ms"
    /// Maximum mmap size per index (bytes).
    pub max_index_size: usize,      // default: 100 GiB
    /// Maximum mmap size for the task database (bytes).
    pub max_task_db_size: usize,    // default: 10 GiB
}
```

Provides `Into<MeilisearchOptions>` for seamless conversion.

### `VectorStoreConfig` (new: `src/config.rs`, feature-gated)

A serde-friendly mirror of `SurrealDbVectorStoreConfig`:

```rust
#[cfg(feature = "surrealdb")]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VectorStoreConfig {
    pub connection_string: String,   // default: "memory"
    pub namespace: String,           // default: "meilisearch"
    pub database: String,            // default: "vectors"
    pub table: String,               // default: "embeddings"
    pub dimensions: usize,           // default: 384
    pub hnsw_m: usize,              // default: 16
    pub hnsw_ef: usize,             // default: 500
    pub quantized: bool,             // default: false
    pub auth: Option<VectorStoreAuth>, // default: None
}

#[cfg(feature = "surrealdb")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorStoreAuth {
    pub username: String,
    pub password: String,
}
```

Provides `Into<SurrealDbVectorStoreConfig>` for seamless conversion.

### `RagConfig` (new: `src/config.rs`)

Serializable subset of `PipelineConfig` scalars:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RagConfig {
    pub retrieval_limit: usize,      // default: 20
    pub rerank_limit: usize,         // default: 5
    pub max_context_chars: usize,    // default: 8000
    pub include_snippets: bool,      // default: true
    /// "keyword", "semantic", or "hybrid"
    pub default_search_type: String, // default: "hybrid"
    /// Semantic weight for hybrid search [0.0, 1.0].
    pub semantic_ratio: f32,         // default: 0.5
}
```

Provides `TryInto<PipelineConfig>` that reconstructs the `SearchType` enum.

### `SearchDefaultsConfig` (new: `src/config.rs`)

Global defaults for search queries:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchDefaultsConfig {
    pub limit: usize,                // default: 20
    pub highlight_pre_tag: String,   // default: "<em>"
    pub highlight_post_tag: String,  // default: "</em>"
    pub crop_length: usize,          // default: 10
    pub crop_marker: String,         // default: "..."
}
```

### Loading Methods

```rust
impl WilysearchConfig {
    /// Standard figment: defaults -> wilysearch.toml -> env vars.
    pub fn figment() -> Figment { ... }

    /// Load from standard figment.
    pub fn load() -> Result<Self, figment::Error> { ... }

    /// Load from specific TOML path + env overrides.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, figment::Error> { ... }

    /// Extract from a consumer-provided Figment.
    pub fn from_figment(figment: Figment) -> Result<Self, figment::Error> { ... }

    /// Validate all values after loading.
    pub fn validate(&self) -> Result<(), ConfigError> { ... }
}
```

### Engine Integration

```rust
impl Engine {
    // Existing (unchanged):
    pub fn new(options: MeilisearchOptions) -> Result<Self> { ... }
    pub fn default_engine() -> Result<Self> { ... }

    // New:
    pub fn with_config(config: WilysearchConfig) -> Result<Self> { ... }
    pub fn from_config_file(path: impl AsRef<Path>) -> Result<Self> { ... }
}
```

`Engine::with_config` internally:
1. Converts `config.engine` -> `MeilisearchOptions` and calls the existing `Meilisearch::new()`
2. Applies `config.experimental` features
3. Stores `config.preprocessing` for query preprocessing
4. Stores `config.search_defaults` for default search parameters
5. If `surrealdb` feature enabled and `config.vector_store` is `Some`, creates the vector store

---

## Environment Variable Convention

| Config Path | Environment Variable |
|-------------|---------------------|
| `engine.db_path` | `WILYSEARCH__ENGINE__DB_PATH` |
| `engine.max_index_size` | `WILYSEARCH__ENGINE__MAX_INDEX_SIZE` |
| `engine.max_task_db_size` | `WILYSEARCH__ENGINE__MAX_TASK_DB_SIZE` |
| `preprocessing.typo.enabled` | `WILYSEARCH__PREPROCESSING__TYPO__ENABLED` |
| `preprocessing.typo.max_edit_distance` | `WILYSEARCH__PREPROCESSING__TYPO__MAX_EDIT_DISTANCE` |
| `preprocessing.synonyms.enabled` | `WILYSEARCH__PREPROCESSING__SYNONYMS__ENABLED` |
| `preprocessing.normalization.lowercase` | `WILYSEARCH__PREPROCESSING__NORMALIZATION__LOWERCASE` |
| `preprocessing.paths.english_dict` | `WILYSEARCH__PREPROCESSING__PATHS__ENGLISH_DICT` |
| `vector_store.connection_string` | `WILYSEARCH__VECTOR_STORE__CONNECTION_STRING` |
| `vector_store.dimensions` | `WILYSEARCH__VECTOR_STORE__DIMENSIONS` |
| `vector_store.auth.username` | `WILYSEARCH__VECTOR_STORE__AUTH__USERNAME` |
| `vector_store.auth.password` | `WILYSEARCH__VECTOR_STORE__AUTH__PASSWORD` |
| `rag.retrieval_limit` | `WILYSEARCH__RAG__RETRIEVAL_LIMIT` |
| `rag.semantic_ratio` | `WILYSEARCH__RAG__SEMANTIC_RATIO` |
| `experimental.metrics` | `WILYSEARCH__EXPERIMENTAL__METRICS` |
| `experimental.contains_filter` | `WILYSEARCH__EXPERIMENTAL__CONTAINS_FILTER` |
| `search_defaults.limit` | `WILYSEARCH__SEARCH_DEFAULTS__LIMIT` |
| `search_defaults.crop_length` | `WILYSEARCH__SEARCH_DEFAULTS__CROP_LENGTH` |

**Rule:** Double-underscore (`__`) separates nesting levels. figment's `Env::prefixed("WILYSEARCH__").split("__")` handles this automatically. Snake_case field names are uppercased by figment.

---

## Error Handling

### Configuration Errors

A new `ConfigError` enum in `src/config.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("configuration error: {0}")]
    Figment(#[from] figment::Error),

    #[error("validation error: {field}: {message} (got: {value})")]
    Validation {
        field: String,
        value: String,
        message: String,
    },
}
```

figment already provides provenance in its errors. The `Validation` variant adds domain-specific checks (e.g., "max_edit_distance must be <= 3").

### Validation Rules

| Field | Constraint | Error Message |
|-------|-----------|---------------|
| `engine.max_index_size` | > 0 | "must be greater than 0" |
| `engine.max_task_db_size` | > 0 | "must be greater than 0" |
| `preprocessing.typo.max_edit_distance` | 0..=3 | "must be between 0 and 3" |
| `vector_store.dimensions` | > 0 (when present) | "must be greater than 0" |
| `rag.semantic_ratio` | 0.0..=1.0 | "must be between 0.0 and 1.0" |
| `rag.default_search_type` | one of "keyword", "semantic", "hybrid" | "must be 'keyword', 'semantic', or 'hybrid'" |

---

## Testing Strategy

### Unit Tests (`src/config.rs`)
- Default config is valid
- TOML deserialization with all sections
- TOML deserialization with partial sections (defaults fill gaps)
- Unknown keys are ignored
- Validation catches invalid values
- `EngineConfig` converts to `MeilisearchOptions` correctly
- `RagConfig` converts to `PipelineConfig` correctly

### Integration Tests (`tests/config_tests.rs`)
- `Engine::with_config(WilysearchConfig::default())` creates a working engine
- `Engine::from_config_file("test.toml")` loads and creates a working engine
- Env var overrides are applied (using `figment::Jail` for hermetic env testing)
- Feature-gated `[vector_store]` section ignored when feature is off
- Invalid config produces error with field name and provenance

### Reference File Test
- `wilysearch.reference.toml` parses successfully
- Loading it produces a config identical to `WilysearchConfig::default()`

---

## File Changes Summary

| File | Change Type | Description |
|------|-------------|-------------|
| `Cargo.toml` | Modified | Add `figment` dependency |
| `src/config.rs` | **New** | `WilysearchConfig` + all section structs + loading + validation |
| `src/lib.rs` | Modified | Add `pub mod config;` and re-export `WilysearchConfig` |
| `src/engine.rs` | Modified | Add `Engine::with_config()` and `Engine::from_config_file()` |
| `src/core/vector/surrealdb.rs` | Modified | Add `Serialize, Deserialize` to `SurrealDbVectorStoreConfig` and `SurrealDbAuth` |
| `wilysearch.reference.toml` | **New** | Annotated reference config with all defaults |
| `tests/config_tests.rs` | **New** | Config loading and validation integration tests |
