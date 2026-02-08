# Unified Configuration Interface -- Tasks

Implementation plan organized foundation-first: dependency setup, config structs, engine integration, documentation, testing.

---

## Phase 1: Foundation

### 1.1 Add `figment` dependency to `Cargo.toml`

Add `figment` with `toml` and `env` features to `[dependencies]`:

```toml
figment = { version = "0.10", features = ["toml", "env"] }
```

Verify: `cargo check` compiles.

_Requirements: R-CFG-01, R-ENV-01, NR-04_

---

### 1.2 Add serde derives to `SurrealDbVectorStoreConfig` and `SurrealDbAuth`

**File:** `src/core/vector/surrealdb.rs`

- Add `Serialize, Deserialize` to `SurrealDbVectorStoreConfig` (currently only has `Debug, Clone`)
- Add `Serialize, Deserialize` to `SurrealDbAuth` (currently only has `Debug, Clone`)
- Add `#[serde(default)]` to `SurrealDbVectorStoreConfig`
- Add `#[serde(skip_serializing_if = "Option::is_none")]` to the `auth` field

This is a prerequisite for including the vector store in the unified config struct.

Verify: `cargo check --features surrealdb` compiles.

_Requirements: R-SEC-01 (vector_store section)_

---

### 1.3 Create `src/config.rs` with all config structs

**File:** `src/config.rs` (new)

Define:
- `WilysearchConfig` -- top-level struct with `#[serde(default)]`
- `EngineConfig` -- wraps MeilisearchOptions fields
- `VectorStoreConfig` + `VectorStoreAuth` -- feature-gated serde mirror of `SurrealDbVectorStoreConfig`
- `RagConfig` -- scalar subset of `PipelineConfig` with flattened `SearchType`
- `SearchDefaultsConfig` -- default search query settings
- `ConfigError` -- thiserror enum for figment + validation errors

Implement:
- `Default` for each config struct (values must match existing subsystem defaults)
- `Into<MeilisearchOptions>` for `EngineConfig`
- `Into<SurrealDbVectorStoreConfig>` for `VectorStoreConfig` (feature-gated)
- `TryInto<PipelineConfig>` for `RagConfig` (reconstructs `SearchType` enum)
- `WilysearchConfig::validate(&self) -> Result<(), ConfigError>`

Do NOT implement loading methods yet -- that's task 1.4.

Verify: `cargo check` compiles; defaults match existing behavior.

_Requirements: R-API-01, R-DEF-01, R-DEF-02, R-SEC-01, R-VAL-01 through R-VAL-06_

---

### 1.4 Implement figment loading methods on `WilysearchConfig`

**File:** `src/config.rs`

Implement:
- `WilysearchConfig::figment() -> Figment` -- standard figment (defaults -> `wilysearch.toml` -> env)
- `WilysearchConfig::load() -> Result<Self, ConfigError>` -- extract from standard figment + validate
- `WilysearchConfig::from_file(path) -> Result<Self, ConfigError>` -- specific file + env + validate
- `WilysearchConfig::from_figment(figment) -> Result<Self, ConfigError>` -- consumer-provided figment + validate
- `impl Provider for WilysearchConfig` -- so consumers can use a config struct as a figment source

Verify: unit tests (task 3.1) pass.

_Requirements: R-CFG-01, R-CFG-04, R-ENV-01, R-ENV-02, R-API-05_

---

### 1.5 Export config module from `lib.rs`

**File:** `src/lib.rs`

- Add `pub mod config;`
- Consider re-exporting `WilysearchConfig` at the crate root for ergonomics

Verify: `cargo doc --no-deps` generates documentation for the config module.

_Requirements: NR-01 (additive, no breaking changes)_

---

## Phase 2: Engine Integration

### 2.1 Add `Engine::with_config(WilysearchConfig)` constructor

**File:** `src/engine.rs`

Implement `Engine::with_config(config: WilysearchConfig) -> Result<Self>`:

1. Convert `config.engine` to `MeilisearchOptions` via `Into`
2. Call existing `Meilisearch::new(options)`
3. Apply `config.experimental` via `meilisearch.update_experimental_features()`
4. Store `config.preprocessing`, `config.rag`, and `config.search_defaults` on the Engine struct (new fields, or stored in an `Arc<WilysearchConfig>`)

Note: The Engine struct may need a new field to hold the full config (or relevant subsections). Evaluate whether to store the entire `WilysearchConfig` or just the parts that are used at query time.

Verify: `Engine::with_config(WilysearchConfig::default())` produces a working engine; all existing tests pass.

_Requirements: R-API-02, R-API-04 (backward compat)_

---

### 2.2 Add `Engine::from_config_file(path)` convenience constructor

**File:** `src/engine.rs`

```rust
pub fn from_config_file(path: impl AsRef<Path>) -> Result<Self> {
    let config = WilysearchConfig::from_file(path)
        .map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)?;
    Self::with_config(config)
}
```

Verify: test creates an engine from a temp TOML file.

_Requirements: R-API-03_

---

### 2.3 Wire vector store configuration (feature-gated)

**File:** `src/engine.rs`

In `Engine::with_config`, when `surrealdb` feature is enabled and `config.vector_store` is `Some`:
1. Convert `VectorStoreConfig` to `SurrealDbVectorStoreConfig`
2. Create `SurrealDbVectorStore::new(config).await?` (requires a tokio runtime block_on or the existing sync wrapper pattern from the surrealdb module)
3. Attach to the `Meilisearch` instance via `with_vector_store()`

Verify: `cargo check --features surrealdb` compiles. Tested in task 3.3.

_Requirements: R-SEC-02, R-SEC-03_

---

## Phase 3: Documentation & Testing

### 3.1 Add unit tests for config structs and loading

**File:** `src/config.rs` (inline `#[cfg(test)]` module)

Tests:
- `test_default_config_is_valid` -- `WilysearchConfig::default().validate()` succeeds
- `test_toml_deserialization_full` -- all sections present
- `test_toml_deserialization_partial` -- some sections omitted, defaults fill in
- `test_toml_unknown_keys_ignored` -- extra keys don't cause errors
- `test_validation_catches_zero_index_size` -- R-VAL-01
- `test_validation_catches_invalid_edit_distance` -- R-VAL-03
- `test_validation_catches_invalid_semantic_ratio` -- R-VAL-05
- `test_validation_catches_invalid_search_type` -- R-VAL-06
- `test_engine_config_to_meilisearch_options` -- conversion preserves values
- `test_rag_config_to_pipeline_config` -- SearchType reconstruction
- `test_rag_config_to_pipeline_config_keyword` -- "keyword" string maps correctly
- `test_rag_config_to_pipeline_config_semantic` -- "semantic" string maps correctly

_Requirements: R-DEF-01, R-VAL-01 through R-VAL-06_

---

### 3.2 Add integration tests for config loading

**File:** `tests/config_tests.rs` (new)

Tests:
- `test_engine_with_default_config` -- `Engine::with_config(WilysearchConfig::default())` works, can create index and search
- `test_engine_from_config_file` -- write temp TOML, load, create engine, verify settings applied
- `test_env_var_overrides` -- use `figment::Jail` to set env vars, verify they override file values
- `test_empty_toml_uses_defaults` -- empty file produces same config as `Default::default()`
- `test_invalid_config_error_message` -- bad value produces error with field name

_Requirements: R-CFG-01, R-CFG-04, R-ENV-01, R-ENV-03, NR-03_

---

### 3.3 Create `wilysearch.reference.toml`

**File:** `wilysearch.reference.toml` (new, project root)

Fully annotated TOML file with every field, its default value, type annotation in a comment, and a one-line description. This serves as both documentation and a quick-start template.

Include a test that parses this file and asserts the result equals `WilysearchConfig::default()`.

_Requirements: NR-05, R-DEF-01_

---

### 3.4 Verify all existing tests pass

Run `cargo test` and `cargo test --features surrealdb` to ensure zero regressions.

_Requirements: NR-01_

---

## Implementation Notes

### Config struct vs. reuse of existing structs

The design creates new config-layer structs (`EngineConfig`, `VectorStoreConfig`, `RagConfig`, `SearchDefaultsConfig`) rather than reusing the existing internal structs directly. Reasons:

1. **`MeilisearchOptions`** -- Already has `Serialize/Deserialize` but the field names don't match TOML conventions (we want `[engine]` not `[meilisearch_options]`). A thin wrapper with `Into` conversion keeps the TOML clean.

2. **`SurrealDbVectorStoreConfig`** -- Does not have `Serialize/Deserialize` (task 1.2 adds them). But it also has an `auth: Option<SurrealDbAuth>` where `SurrealDbAuth` needs serde. A config-layer mirror avoids forcing serde on the vector store's internal types.

3. **`PipelineConfig`** -- Contains `SearchType` enum with payload that doesn't map cleanly to TOML. The config-layer `RagConfig` flattens this to two scalar fields.

4. **`PreprocessingConfig`** -- Already has full serde support and `from_toml()`. We can reuse it directly as a nested section.

5. **`ExperimentalFeatures`** -- Already has full serde support. Reuse directly.

### Engine struct changes

`Engine` currently holds:
```rust
inner: Meilisearch,
task_counter: AtomicU64,
dump_dir: PathBuf,
snapshot_dir: PathBuf,
```

After this work, it may additionally store:
```rust
search_defaults: SearchDefaultsConfig,  // applied as base for SearchQuery construction
preprocessing_config: PreprocessingConfig,  // passed to preprocessing pipeline
rag_config: RagConfig,  // defaults for RAG pipeline construction
```

The exact integration of these stored configs with query-time behavior is a follow-up concern -- this spec focuses on getting the config *loaded and validated*. The stored values become useful when the preprocessing pipeline and RAG pipeline are wired into the search path.

### Dependency graph

```
Task 1.1 (add figment dep)
    |
    +--> Task 1.3 (config structs)
    |        |
    |        +--> Task 1.4 (loading methods)
    |                |
    |                +--> Task 1.5 (lib.rs export)
    |                        |
    |                        +--> Task 2.1 (Engine::with_config)
    |                                |
    |                                +--> Task 2.2 (Engine::from_config_file)
    |                                |
    |                                +--> Task 2.3 (wire vector store)
    |
    +--> Task 1.2 (serde on SurrealDb types)
             |
             +--> Task 2.3 (wire vector store)

Task 3.1 (unit tests) -- can start after Task 1.4
Task 3.2 (integration tests) -- can start after Task 2.2
Task 3.3 (reference.toml) -- can start after Task 1.3
Task 3.4 (verify existing tests) -- after all changes
```
