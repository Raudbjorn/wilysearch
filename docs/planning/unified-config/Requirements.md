# Unified Configuration Interface -- Requirements

**Feature:** Single-file, environment-overridable configuration for all wilysearch subsystems.

**Problem Statement:** wilysearch is a library crate with no binary -- consumers cannot use CLI flags like `meilisearch --db-path ./data`. Configuration is currently scattered across multiple entry points (`MeilisearchOptions`, `PreprocessingConfig`, `SurrealDbVectorStoreConfig`, `PipelineConfig`, `ExperimentalFeatures`) with no unified loading mechanism, no environment variable support, and no single-file configuration story. Each subsystem must be configured independently in Rust code, making deployment-time tuning (containers, CI, different environments) cumbersome.

---

## User Stories

### US-1: Library Consumer (Programmatic)
As a Rust developer embedding wilysearch, I want to construct a single typed config struct that covers all subsystems, so that I can configure the entire engine in one place without hunting through multiple option types.

### US-2: Application Developer (File-Based)
As a developer deploying an application that uses wilysearch, I want to load all configuration from a single TOML file, so that I can manage settings declaratively without recompilation.

### US-3: DevOps / Container Deployment
As an operator deploying a wilysearch-backed application, I want to override any configuration value via environment variables, so that I can tune settings per environment (dev/staging/prod) without modifying config files, and keep secrets out of files.

### US-4: Power User (Custom Layering)
As an advanced consumer, I want to compose multiple config sources (defaults < base file < environment-specific file < env vars) with clear precedence, so that I can build sophisticated deployment configurations.

---

## Functional Requirements

### Configuration File

**R-CFG-01:** WHEN a consumer calls `WilysearchConfig::from_file(path)` THEN the system SHALL load all configuration from the specified TOML file, applying defaults for any omitted field.

**R-CFG-02:** WHEN the TOML file contains an unknown key THEN the system SHALL ignore it without error (forward-compatible).

**R-CFG-03:** WHEN the TOML file is syntactically invalid THEN the system SHALL return an error identifying the file path and the parse error location.

**R-CFG-04:** WHEN a consumer calls `WilysearchConfig::load()` with no arguments THEN the system SHALL attempt to load from `wilysearch.toml` in the current directory, applying defaults if the file does not exist.

### Environment Variable Overrides

**R-ENV-01:** WHEN an environment variable prefixed with `WILYSEARCH__` is set THEN the system SHALL use its value to override the corresponding config field, taking precedence over file values.

**R-ENV-02:** The environment variable naming convention SHALL use double-underscore (`__`) to separate nesting levels:
- `WILYSEARCH__ENGINE__DB_PATH` maps to `[engine].db_path`
- `WILYSEARCH__PREPROCESSING__TYPO__ENABLED` maps to `[preprocessing.typo].enabled`
- `WILYSEARCH__VECTOR_STORE__AUTH__PASSWORD` maps to `[vector_store.auth].password`

**R-ENV-03:** WHEN an environment variable contains a value that cannot be parsed as the expected type THEN the system SHALL return an error identifying the environment variable name and the expected type.

### Programmatic API

**R-API-01:** The system SHALL provide a `WilysearchConfig` struct that can be constructed directly in Rust code without any file or environment variable, using `Default` for all omitted fields.

**R-API-02:** The system SHALL provide `Engine::with_config(config: WilysearchConfig)` as a constructor that wires all subsystem configuration from the unified struct.

**R-API-03:** The system SHALL provide `Engine::from_config_file(path)` as a convenience constructor that loads a TOML file, applies env overrides, and constructs the engine.

**R-API-04:** The existing `Engine::new(MeilisearchOptions)` and `Engine::default_engine()` constructors SHALL continue to work unchanged (backward compatibility).

**R-API-05:** The system SHALL provide `WilysearchConfig::figment()` returning a composable config builder that consumers can extend with additional sources before extraction.

### Configuration Sections

**R-SEC-01:** The unified configuration SHALL include the following top-level sections:

| Section | Maps To | Required |
|---------|---------|----------|
| `[engine]` | `MeilisearchOptions` | No (has defaults) |
| `[preprocessing]` | `PreprocessingConfig` | No (has defaults) |
| `[preprocessing.typo]` | `TypoConfig` | No (has defaults) |
| `[preprocessing.synonyms]` | `SynonymConfig` | No (has defaults) |
| `[preprocessing.normalization]` | `NormalizationConfig` | No (has defaults) |
| `[preprocessing.paths]` | `DictionaryPaths` | No (has defaults) |
| `[vector_store]` | `SurrealDbVectorStoreConfig` | No (feature-gated, optional) |
| `[vector_store.auth]` | `SurrealDbAuth` | No (optional within vector_store) |
| `[rag]` | `PipelineConfig` scalar fields | No (has defaults) |
| `[experimental]` | `ExperimentalFeatures` | No (all default false) |
| `[search_defaults]` | Default search tuning | No (has defaults) |

**R-SEC-02:** WHEN the `surrealdb` feature is not enabled THEN the `[vector_store]` section SHALL be silently ignored if present in the config file.

**R-SEC-03:** WHEN the `surrealdb` feature is enabled AND no `[vector_store]` section is present THEN the vector store SHALL default to `None` (no vector store configured).

### Defaults

**R-DEF-01:** Every configuration field SHALL have a documented default value. A completely empty TOML file (or no file at all) SHALL produce a valid, working configuration.

**R-DEF-02:** Default values SHALL match the current behavior of each subsystem's `Default` impl, preserving backward compatibility.

### Validation

**R-VAL-01:** WHEN `max_index_size` is zero THEN the system SHALL return a validation error.

**R-VAL-02:** WHEN `max_task_db_size` is zero THEN the system SHALL return a validation error.

**R-VAL-03:** WHEN `preprocessing.typo.max_edit_distance` is greater than 3 THEN the system SHALL return a validation error.

**R-VAL-04:** WHEN `vector_store.dimensions` is zero THEN the system SHALL return a validation error.

**R-VAL-05:** WHEN `rag.semantic_ratio` is outside `[0.0, 1.0]` THEN the system SHALL return a validation error.

**R-VAL-06:** WHEN a validation error occurs THEN the error message SHALL identify the field name, the invalid value, and the constraint that was violated.

---

## Non-Functional Requirements

**NR-01 (Backward Compatibility):** The existing public API (`Engine::new`, `Engine::default_engine`, all 10 traits, all 48+ public types) SHALL remain unchanged. The unified config is purely additive.

**NR-02 (Zero Runtime Cost for Unused Features):** The `figment` dependency SHALL not pull in unnecessary runtime allocations when consumers use the programmatic API path (direct struct construction) without file or env loading.

**NR-03 (Error Provenance):** Configuration errors SHALL identify which source (TOML file, environment variable, or programmatic default) provided the invalid value. This is critical for debugging deployment issues.

**NR-04 (Minimal Dependencies):** The unified config system SHALL add at most one new direct dependency (`figment`) with its `toml` and `env` provider features.

**NR-05 (Documentation):** The crate SHALL include a reference TOML file (`wilysearch.reference.toml`) with every field, its default value, type, and a one-line description as a TOML comment.

---

## Edge Cases

**EC-01:** Config file exists but is completely empty -> all defaults applied, engine works.

**EC-02:** Env var set for a field that also appears in the TOML file -> env var wins.

**EC-03:** Env var set for a field in a feature-gated section (`vector_store`) when the feature is disabled -> silently ignored.

**EC-04:** TOML file specifies `[vector_store.auth]` without `username`/`password` -> defaults to empty strings (SurrealDB will reject at connection time, not at config time).

**EC-05:** Consumer constructs `WilysearchConfig` programmatically and passes it to `Engine::with_config` -- no file or env vars are read.

**EC-06:** `db_path` set via env var as a relative path -> used as-is (resolved relative to CWD at engine construction time, same as current behavior).

---

## Out of Scope

- CLI flag parsing (wilysearch is a library, not a binary)
- Hot-reloading configuration at runtime
- Remote configuration sources (Consul, etcd, etc.)
- Per-index settings in the config file (those are managed via the Settings trait at runtime)
- Per-query search parameters (those are per-request, not global config)
- YAML/JSON config file support (TOML only for v1; figment makes adding these trivial later)
