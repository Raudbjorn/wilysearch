# Configuration Guide

## Overview

wilysearch supports unified configuration through three mechanisms:

1. **Programmatic construction** -- build a `WilysearchConfig` struct directly in Rust code
2. **TOML file** -- load settings from a declarative configuration file
3. **Environment variables** -- override any setting at deploy time

Configuration uses the [figment](https://docs.rs/figment) crate for layered loading with provenance-aware error messages. Sources are applied with later sources taking precedence:

```
defaults < TOML file < environment variables < programmatic overrides
```

Every field has a documented default. A completely empty TOML file (or no file at all) produces a valid, working configuration.

---

## Quick Start

### Programmatic (struct literal)

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

### File-based (TOML)

Create a `wilysearch.toml`:

```toml
[engine]
db_path = "/var/lib/wilysearch"
max_index_size = 53_687_091_200   # 50 GiB

[preprocessing.typo]
enabled = true
maxEditDistance = 2

[search_defaults]
limit = 50
```

Load it:

```rust
use wilysearch::engine::Engine;

let engine = Engine::from_config_file("wilysearch.toml")?;
```

### Environment variables

```bash
export WILYSEARCH__ENGINE__DB_PATH=/var/lib/wilysearch
export WILYSEARCH__ENGINE__MAX_INDEX_SIZE=53687091200
export WILYSEARCH__SEARCH_DEFAULTS__LIMIT=50
```

```rust
use wilysearch::config::WilysearchConfig;
use wilysearch::engine::Engine;

// Loads from wilysearch.toml (if present) + env var overrides
let config = WilysearchConfig::load()?;
let engine = Engine::with_config(config)?;
```

---

## Loading Configuration

### `WilysearchConfig::default()`

Returns a configuration with all compiled-in defaults. No file is read and no environment variables are consulted.

```rust
let config = WilysearchConfig::default();
```

### `WilysearchConfig::load()`

Loads from the standard layering: defaults, then `wilysearch.toml` in the current directory (if it exists), then `WILYSEARCH__*` environment variables. Validates all values before returning.

```rust
let config = WilysearchConfig::load()?;
```

### `WilysearchConfig::from_file(path)`

Loads from a specific TOML file path instead of the default `wilysearch.toml`. Environment variable overrides are still applied on top.

```rust
let config = WilysearchConfig::from_file("/etc/myapp/search.toml")?;
```

### `WilysearchConfig::figment()`

Returns the raw `Figment` builder before extraction, allowing consumers to insert additional sources. This is the power-user escape hatch for custom layering.

```rust
use figment::providers::{Format, Toml};

let figment = WilysearchConfig::figment()
    .merge(Toml::file("overrides.toml"));
let config = WilysearchConfig::from_figment(figment)?;
```

### `Engine::with_config(config)`

Constructs an `Engine` from a `WilysearchConfig`. This is the primary constructor when you have a config in hand.

```rust
let config = WilysearchConfig::load()?;
let engine = Engine::with_config(config)?;
```

Internally this:
1. Converts `config.engine` to `MeilisearchOptions` and creates the LMDB-backed `Meilisearch` instance
2. If the `surrealdb` feature is enabled and `config.vector_store` is `Some`, creates and attaches the SurrealDB vector store
3. Stores preprocessing, RAG, experimental, and search default settings for runtime use

### `Engine::from_config_file(path)`

Convenience method that combines `WilysearchConfig::from_file(path)` and `Engine::with_config()` in a single call.

```rust
let engine = Engine::from_config_file("wilysearch.toml")?;
```

---

## Configuration Reference

### `[engine]`

LMDB engine settings controlling database location and memory-map sizes.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `db_path` | PathBuf | `"data.ms"` | Directory for LMDB database files |
| `max_index_size` | usize (bytes) | 107,374,182,400 (100 GiB) | Maximum mmap size per index |
| `max_task_db_size` | usize (bytes) | 10,737,418,240 (10 GiB) | Maximum mmap size for the task database |

```toml
[engine]
db_path = "/var/lib/wilysearch"
max_index_size = 107_374_182_400
max_task_db_size = 10_737_418_240
```

---

### `[preprocessing]`

Query preprocessing pipeline configuration. Contains four sub-sections: `typo`, `synonyms`, `paths`, and `normalization`.

---

### `[preprocessing.typo]`

SymSpell-based typo correction settings. Field names use **camelCase** in TOML (matching the serde `rename_all = "camelCase"` on `TypoConfig`).

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `true` | Enable typo correction globally |
| `minWordSizeOneTypo` | usize | 5 | Minimum word length for single-typo tolerance |
| `minWordSizeTwoTypos` | usize | 9 | Minimum word length for two-typo tolerance |
| `maxEditDistance` | i64 | 2 | Maximum edit distance for corrections (0--3) |
| `disabledOnWords` | \[String\] | `[]` | Words to skip typo correction for (domain terms, brand names) |

```toml
[preprocessing.typo]
enabled = true
minWordSizeOneTypo = 5
minWordSizeTwoTypos = 9
maxEditDistance = 2
disabledOnWords = ["dnd", "5e", "phb"]
```

---

### `[preprocessing.synonyms]`

Synonym expansion settings. Field names use **camelCase** in TOML.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `true` | Enable synonym expansion |
| `maxExpansions` | usize | 10 | Maximum synonyms generated per query term |
| `includeOriginal` | bool | `true` | Keep the original term alongside its expansions |

```toml
[preprocessing.synonyms]
enabled = true
maxExpansions = 10
includeOriginal = true
```

---

### `[preprocessing.normalization]`

Text normalization applied before typo correction and synonym expansion. Field names use **snake_case** in TOML.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `lowercase` | bool | `true` | Convert text to lowercase |
| `trim` | bool | `true` | Trim leading and trailing whitespace |
| `collapse_whitespace` | bool | `true` | Collapse multiple spaces into one |
| `unicode_normalize` | bool | `false` | Apply NFKC Unicode normalization |

```toml
[preprocessing.normalization]
lowercase = true
trim = true
collapse_whitespace = true
unicode_normalize = false
```

---

### `[preprocessing.paths]`

Paths to dictionary and synonym files used by the preprocessing pipeline. All paths are optional; omitting a path disables the corresponding file-based feature. Field names use **snake_case** in TOML.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `english_dict` | PathBuf? | `None` | Path to English frequency dictionary |
| `corpus_dict` | PathBuf? | `None` | Path to corpus-specific frequency dictionary |
| `bigram_dict` | PathBuf? | `None` | Path to bigram dictionary (compound word correction) |
| `synonyms_file` | PathBuf? | `None` | Path to synonyms TOML file |

```toml
[preprocessing.paths]
english_dict = "data/frequency_dictionary_en.txt"
corpus_dict = "data/corpus.txt"
bigram_dict = "data/bigrams.txt"
synonyms_file = "data/synonyms.toml"
```

Dictionary file formats:

- **Frequency dictionaries** (`english_dict`, `corpus_dict`): one entry per line, `word frequency` separated by a space.
  ```
  the 23135851162
  of 13151942776
  ```
- **Bigram dictionary** (`bigram_dict`): one entry per line, `word1 word2 frequency` separated by spaces.
  ```
  in the 123456
  of the 98765
  ```
- **Synonyms file** (`synonyms_file`): a TOML file following the synonym configuration format.

---

### `[rag]`

RAG (Retrieval-Augmented Generation) pipeline defaults.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `retrieval_limit` | usize | 20 | Number of documents to retrieve before reranking |
| `rerank_limit` | usize | 5 | Number of documents to keep after reranking |
| `max_context_chars` | usize | 8000 | Maximum context length in characters for the generator |
| `include_snippets` | bool | `true` | Include source snippets in the response |
| `default_search_type` | String | `"hybrid"` | Search strategy: `"keyword"`, `"semantic"`, or `"hybrid"` |
| `semantic_ratio` | f32 | 0.5 | Semantic weight for hybrid search \[0.0, 1.0\]; only used when `default_search_type = "hybrid"` |

```toml
[rag]
retrieval_limit = 20
rerank_limit = 5
max_context_chars = 8000
include_snippets = true
default_search_type = "hybrid"
semantic_ratio = 0.5
```

---

### `[experimental]`

Experimental feature flags. All default to `false`. These correspond to experimental features in the upstream Meilisearch engine.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `metrics` | bool | `false` | Enable Prometheus metrics |
| `logs_route` | bool | `false` | Enable real-time log streaming |
| `edit_documents_by_function` | bool | `false` | Enable JS-based document editing |
| `contains_filter` | bool | `false` | Enable the `CONTAINS` filter operator |
| `composite_embedders` | bool | `false` | Enable multi-source embedders |
| `multimodal` | bool | `false` | Enable multimodal embeddings |
| `vector_store_setting` | bool | `false` | Enable vector store in index configuration |

```toml
[experimental]
metrics = false
logs_route = false
edit_documents_by_function = false
contains_filter = false
composite_embedders = false
multimodal = false
vector_store_setting = false
```

---

### `[search_defaults]`

Default parameters applied to search queries when not explicitly specified in the `SearchRequest`.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `limit` | usize | 20 | Default result limit |
| `highlight_pre_tag` | String | `"<em>"` | HTML tag inserted before highlighted terms |
| `highlight_post_tag` | String | `"</em>"` | HTML tag inserted after highlighted terms |
| `crop_length` | usize | 10 | Default crop length in words |
| `crop_marker` | String | `"..."` | Marker inserted at crop boundaries |

```toml
[search_defaults]
limit = 20
highlight_pre_tag = "<em>"
highlight_post_tag = "</em>"
crop_length = 10
crop_marker = "..."
```

---

### `[vector_store]` (feature-gated)

SurrealDB vector store configuration. This section is only available when the `surrealdb` feature is enabled. When the feature is disabled, any `[vector_store]` section in the TOML file is silently ignored.

When the feature is enabled but no `[vector_store]` section is present, no vector store is configured (`None`).

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `connection_string` | String | `"memory"` | SurrealDB connection string (`"memory"`, `"file:///path"`, `"ws://host:port"`) |
| `namespace` | String | `"meilisearch"` | SurrealDB namespace |
| `database` | String | `"vectors"` | SurrealDB database name |
| `table` | String | `"embeddings"` | Table name for storing vectors |
| `dimensions` | usize | 384 | Vector dimensions |
| `hnsw_m` | usize | 16 | HNSW M parameter (max connections per node) |
| `hnsw_ef` | usize | 500 | HNSW EF construction parameter |
| `quantized` | bool | `false` | Enable binary quantization |

```toml
[vector_store]
connection_string = "memory"
namespace = "meilisearch"
database = "vectors"
table = "embeddings"
dimensions = 384
hnsw_m = 16
hnsw_ef = 500
quantized = false
```

#### `[vector_store.auth]`

Optional authentication credentials for SurrealDB. Omit the entire section for unauthenticated connections.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `username` | String | -- | SurrealDB username |
| `password` | String | -- | SurrealDB password |

```toml
[vector_store.auth]
username = "admin"
password = "secret"
```

> **Security note:** Prefer setting credentials via environment variables (`WILYSEARCH__VECTOR_STORE__AUTH__USERNAME`, `WILYSEARCH__VECTOR_STORE__AUTH__PASSWORD`) rather than storing them in config files.

---

## Environment Variables

All configuration fields can be overridden via environment variables using the naming convention:

```
WILYSEARCH__<SECTION>__<FIELD>
```

Double-underscore (`__`) separates nesting levels. Field names are uppercased and use snake_case. figment's `Env::prefixed("WILYSEARCH__").split("__")` handles the mapping automatically.

### Full Mapping Table

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
| `preprocessing.paths.corpus_dict` | `WILYSEARCH__PREPROCESSING__PATHS__CORPUS_DICT` |
| `preprocessing.paths.bigram_dict` | `WILYSEARCH__PREPROCESSING__PATHS__BIGRAM_DICT` |
| `preprocessing.paths.synonyms_file` | `WILYSEARCH__PREPROCESSING__PATHS__SYNONYMS_FILE` |
| `rag.retrieval_limit` | `WILYSEARCH__RAG__RETRIEVAL_LIMIT` |
| `rag.rerank_limit` | `WILYSEARCH__RAG__RERANK_LIMIT` |
| `rag.max_context_chars` | `WILYSEARCH__RAG__MAX_CONTEXT_CHARS` |
| `rag.include_snippets` | `WILYSEARCH__RAG__INCLUDE_SNIPPETS` |
| `rag.default_search_type` | `WILYSEARCH__RAG__DEFAULT_SEARCH_TYPE` |
| `rag.semantic_ratio` | `WILYSEARCH__RAG__SEMANTIC_RATIO` |
| `experimental.metrics` | `WILYSEARCH__EXPERIMENTAL__METRICS` |
| `experimental.logs_route` | `WILYSEARCH__EXPERIMENTAL__LOGS_ROUTE` |
| `experimental.edit_documents_by_function` | `WILYSEARCH__EXPERIMENTAL__EDIT_DOCUMENTS_BY_FUNCTION` |
| `experimental.contains_filter` | `WILYSEARCH__EXPERIMENTAL__CONTAINS_FILTER` |
| `experimental.composite_embedders` | `WILYSEARCH__EXPERIMENTAL__COMPOSITE_EMBEDDERS` |
| `experimental.multimodal` | `WILYSEARCH__EXPERIMENTAL__MULTIMODAL` |
| `experimental.vector_store_setting` | `WILYSEARCH__EXPERIMENTAL__VECTOR_STORE_SETTING` |
| `search_defaults.limit` | `WILYSEARCH__SEARCH_DEFAULTS__LIMIT` |
| `search_defaults.highlight_pre_tag` | `WILYSEARCH__SEARCH_DEFAULTS__HIGHLIGHT_PRE_TAG` |
| `search_defaults.highlight_post_tag` | `WILYSEARCH__SEARCH_DEFAULTS__HIGHLIGHT_POST_TAG` |
| `search_defaults.crop_length` | `WILYSEARCH__SEARCH_DEFAULTS__CROP_LENGTH` |
| `search_defaults.crop_marker` | `WILYSEARCH__SEARCH_DEFAULTS__CROP_MARKER` |
| `vector_store.connection_string` | `WILYSEARCH__VECTOR_STORE__CONNECTION_STRING` |
| `vector_store.namespace` | `WILYSEARCH__VECTOR_STORE__NAMESPACE` |
| `vector_store.database` | `WILYSEARCH__VECTOR_STORE__DATABASE` |
| `vector_store.table` | `WILYSEARCH__VECTOR_STORE__TABLE` |
| `vector_store.dimensions` | `WILYSEARCH__VECTOR_STORE__DIMENSIONS` |
| `vector_store.hnsw_m` | `WILYSEARCH__VECTOR_STORE__HNSW_M` |
| `vector_store.hnsw_ef` | `WILYSEARCH__VECTOR_STORE__HNSW_EF` |
| `vector_store.quantized` | `WILYSEARCH__VECTOR_STORE__QUANTIZED` |
| `vector_store.auth.username` | `WILYSEARCH__VECTOR_STORE__AUTH__USERNAME` |
| `vector_store.auth.password` | `WILYSEARCH__VECTOR_STORE__AUTH__PASSWORD` |

### Example: Container Deployment

```bash
docker run -e WILYSEARCH__ENGINE__DB_PATH=/data/search \
           -e WILYSEARCH__ENGINE__MAX_INDEX_SIZE=53687091200 \
           -e WILYSEARCH__RAG__DEFAULT_SEARCH_TYPE=keyword \
           -e WILYSEARCH__SEARCH_DEFAULTS__LIMIT=50 \
           my-app:latest
```

---

## Validation Rules

`WilysearchConfig::validate()` is called automatically by all loading methods (`load()`, `from_file()`, `from_figment()`). It enforces the following constraints:

| Field | Constraint | Error |
|-------|-----------|-------|
| `engine.max_index_size` | Must be > 0 | `"must be greater than 0"` |
| `engine.max_task_db_size` | Must be > 0 | `"must be greater than 0"` |
| `preprocessing.typo.max_edit_distance` | Must be 0--3 | `"must be between 0 and 3"` |
| `rag.semantic_ratio` | Must be in \[0.0, 1.0\] | `"must be between 0.0 and 1.0"` |
| `rag.default_search_type` | Must be `"keyword"`, `"semantic"`, or `"hybrid"` | `"must be 'keyword', 'semantic', or 'hybrid'"` |
| `vector_store.dimensions` | Must be > 0 (when `surrealdb` feature enabled and section present) | `"must be greater than 0"` |

The preprocessing subsystem performs additional validation when `PreprocessingConfig::validate(check_paths)` is called:

| Field | Constraint | Error |
|-------|-----------|-------|
| `preprocessing.typo.min_word_size_one_typo` | Must be <= `min_word_size_two_typos` | `"min_word_size_one_typo must be <= min_word_size_two_typos"` |
| `preprocessing.typo.max_edit_distance` | Must be 0--3 | `"max_edit_distance must be between 0 and 3"` |
| Dictionary paths (when `check_paths = true`) | Must exist on disk | `"Dictionary not found: <path>"` |

---

## Error Messages

Configuration errors include provenance information identifying which source provided the invalid value. There are two error variants:

### `ConfigError::Figment` -- Parse and type errors

These come from the figment framework and include the source (file path, environment variable name, or default):

```
configuration error: invalid type: found string "abc", expected usize
  for key "engine.max_index_size" in env var `WILYSEARCH__ENGINE__MAX_INDEX_SIZE`
```

```
configuration error: invalid type: found boolean true, expected string
  for key "rag.default_search_type" in TOML file `wilysearch.toml`
```

### `ConfigError::Validation` -- Domain-specific constraint violations

These are raised after successful parsing when a value is syntactically valid but violates a domain constraint:

```
validation error: engine.max_index_size: must be greater than 0 (got: 0)
```

```
validation error: rag.semantic_ratio: must be between 0.0 and 1.0 (got: 1.5)
```

```
validation error: rag.default_search_type: must be 'keyword', 'semantic', or 'hybrid' (got: vector)
```

### Handling errors in code

```rust
use wilysearch::config::{WilysearchConfig, ConfigError};

match WilysearchConfig::from_file("wilysearch.toml") {
    Ok(config) => { /* use config */ }
    Err(ConfigError::Figment(e)) => {
        eprintln!("Configuration parse error: {e}");
    }
    Err(ConfigError::Validation { field, value, message }) => {
        eprintln!("Invalid value for {field}: {message} (got: {value})");
    }
}
```

---

## Complete Example TOML

A full configuration file with all sections and their defaults:

```toml
[engine]
db_path = "data.ms"
max_index_size = 107_374_182_400   # 100 GiB
max_task_db_size = 10_737_418_240  # 10 GiB

[preprocessing.typo]
enabled = true
minWordSizeOneTypo = 5
minWordSizeTwoTypos = 9
maxEditDistance = 2
disabledOnWords = []

[preprocessing.synonyms]
enabled = true
maxExpansions = 10
includeOriginal = true

[preprocessing.normalization]
lowercase = true
trim = true
collapse_whitespace = true
unicode_normalize = false

[preprocessing.paths]
# english_dict = "data/frequency_dictionary_en.txt"
# corpus_dict = "data/corpus.txt"
# bigram_dict = "data/bigrams.txt"
# synonyms_file = "data/synonyms.toml"

[rag]
retrieval_limit = 20
rerank_limit = 5
max_context_chars = 8000
include_snippets = true
default_search_type = "hybrid"
semantic_ratio = 0.5

[experimental]
metrics = false
logs_route = false
edit_documents_by_function = false
contains_filter = false
composite_embedders = false
multimodal = false
vector_store_setting = false

[search_defaults]
limit = 20
highlight_pre_tag = "<em>"
highlight_post_tag = "</em>"
crop_length = 10
crop_marker = "..."

# Requires the `surrealdb` feature flag.
# [vector_store]
# connection_string = "memory"
# namespace = "meilisearch"
# database = "vectors"
# table = "embeddings"
# dimensions = 384
# hnsw_m = 16
# hnsw_ef = 500
# quantized = false
#
# [vector_store.auth]
# username = "admin"
# password = "secret"
```
