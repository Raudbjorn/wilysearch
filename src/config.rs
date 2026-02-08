//! Unified configuration for all wilysearch subsystems.
//!
//! This module provides [`WilysearchConfig`] -- a single typed struct that covers
//! engine settings, preprocessing, RAG pipeline, vector store, experimental
//! features, and search defaults. Configuration can be loaded from:
//!
//! 1. **Programmatic construction** -- build the struct directly in Rust code
//! 2. **TOML file** -- `WilysearchConfig::from_file("wilysearch.toml")`
//! 3. **Environment variables** -- `WILYSEARCH__ENGINE__DB_PATH` overrides `[engine].db_path`
//!
//! Sources are layered with later sources taking precedence:
//! defaults < TOML file < environment variables < programmatic overrides.
//!
//! # Example
//!
//! ```no_run
//! use wilysearch::config::WilysearchConfig;
//! use wilysearch::engine::Engine;
//!
//! // Load from file + env overrides
//! let config = WilysearchConfig::from_file("wilysearch.toml").unwrap();
//! let engine = Engine::with_config(config).unwrap();
//!
//! // Or construct programmatically
//! let config = WilysearchConfig {
//!     engine: wilysearch::config::EngineConfig {
//!         db_path: "/tmp/my-db".into(),
//!         ..Default::default()
//!     },
//!     ..Default::default()
//! };
//! let engine = Engine::with_config(config).unwrap();
//! ```

use std::path::{Path, PathBuf};

use figment::providers::{Env, Format, Serialized, Toml};
use figment::{Figment, Provider};
use serde::{Deserialize, Serialize};

use crate::core::preprocessing::config::PreprocessingConfig;
use crate::core::MeilisearchOptions;

// ─── Error type ──────────────────────────────────────────────────────────────

/// Errors that can occur during configuration loading or validation.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Error from the figment configuration framework (parse errors, type
    /// mismatches, missing fields). Includes provenance information showing
    /// which source (file, env var, default) provided the invalid value.
    #[error("configuration error: {0}")]
    Figment(#[from] figment::Error),

    /// A value was syntactically valid but failed domain-specific validation.
    #[error("validation error: {field}: {message} (got: {value})")]
    Validation {
        field: String,
        value: String,
        message: String,
    },
}

// ─── Top-level config ────────────────────────────────────────────────────────

/// Unified configuration for all wilysearch subsystems.
///
/// Every field has a documented default. A completely empty TOML file (or no
/// file at all) produces a valid, working configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WilysearchConfig {
    /// LMDB engine settings (database path, mmap sizes).
    pub engine: EngineConfig,

    /// Query preprocessing pipeline (typo correction, synonyms, normalization).
    pub preprocessing: PreprocessingConfig,

    /// SurrealDB vector store configuration.
    ///
    /// Only present when the `surrealdb` feature is enabled. `None` means
    /// no vector store is configured.
    ///
    /// # Semver note
    ///
    /// This field is `#[cfg]`-gated, so the struct's serialized form differs
    /// between `default` and `surrealdb` feature sets. TOML/JSON configs
    /// written with the `surrealdb` feature enabled will contain a
    /// `vector_store` key that is silently ignored (not rejected) when
    /// deserialized without the feature, thanks to `#[serde(default)]` on
    /// the parent struct. However, **Rust code** that names this field will
    /// fail to compile without the feature. Consumers sharing configs across
    /// feature boundaries should be aware of this asymmetry.
    #[cfg(feature = "surrealdb")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_store: Option<VectorStoreConfig>,

    /// RAG pipeline defaults (retrieval limits, search type, context size).
    pub rag: RagConfig,

    /// Experimental feature flags.
    pub experimental: ExperimentalConfig,

    /// Default search query parameters (limit, highlighting, cropping).
    pub search_defaults: SearchDefaultsConfig,
}

impl Default for WilysearchConfig {
    fn default() -> Self {
        Self {
            engine: EngineConfig::default(),
            preprocessing: PreprocessingConfig::default(),
            #[cfg(feature = "surrealdb")]
            vector_store: None,
            rag: RagConfig::default(),
            experimental: ExperimentalConfig::default(),
            search_defaults: SearchDefaultsConfig::default(),
        }
    }
}

// ─── Engine config ───────────────────────────────────────────────────────────

/// LMDB engine configuration.
///
/// Maps to [`MeilisearchOptions`] via `Into`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineConfig {
    /// Directory where the LMDB database files are stored.
    /// Default: `"data.ms"`
    pub db_path: PathBuf,

    /// Maximum memory-map size per index, in bytes.
    /// Default: 107,374,182,400 (100 GiB)
    pub max_index_size: usize,

    /// Maximum memory-map size for the task database, in bytes.
    /// Default: 10,737,418,240 (10 GiB)
    pub max_task_db_size: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        let opts = MeilisearchOptions::default();
        Self {
            db_path: opts.db_path,
            max_index_size: opts.max_index_size,
            max_task_db_size: opts.max_task_db_size,
        }
    }
}

impl From<EngineConfig> for MeilisearchOptions {
    fn from(c: EngineConfig) -> Self {
        Self {
            db_path: c.db_path,
            max_index_size: c.max_index_size,
            max_task_db_size: c.max_task_db_size,
        }
    }
}

// ─── Vector store config (feature-gated) ─────────────────────────────────────

/// SurrealDB vector store configuration.
///
/// This is a serde-friendly mirror of `SurrealDbVectorStoreConfig`. Use
/// `Into<SurrealDbVectorStoreConfig>` to convert for engine construction.
#[cfg(feature = "surrealdb")]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VectorStoreConfig {
    /// Connection string for SurrealDB.
    /// Examples: `"memory"`, `"file:///path/to/db"`, `"ws://localhost:8000"`
    /// Default: `"memory"`
    pub connection_string: String,

    /// SurrealDB namespace. Default: `"meilisearch"`
    pub namespace: String,

    /// SurrealDB database name. Default: `"vectors"`
    pub database: String,

    /// Table name for storing vectors. Default: `"embeddings"`
    pub table: String,

    /// Vector dimensions. Default: 384
    pub dimensions: usize,

    /// HNSW M parameter (max connections per node). Default: 16
    pub hnsw_m: usize,

    /// HNSW EF construction parameter. Default: 500
    pub hnsw_ef: usize,

    /// Optional authentication credentials.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<VectorStoreAuth>,
}

#[cfg(feature = "surrealdb")]
impl Default for VectorStoreConfig {
    fn default() -> Self {
        Self {
            connection_string: "memory".to_string(),
            namespace: "meilisearch".to_string(),
            database: "vectors".to_string(),
            table: "embeddings".to_string(),
            dimensions: 384,
            hnsw_m: 16,
            hnsw_ef: 500,
            auth: None,
        }
    }
}

/// Authentication credentials for SurrealDB.
///
/// `Serialize` is intentionally NOT derived to prevent accidental credential
/// leakage in logs, debug output, or config round-tripping. Deserialization
/// is still supported for loading from TOML/env.
#[cfg(feature = "surrealdb")]
#[derive(Debug, Clone, Deserialize)]
pub struct VectorStoreAuth {
    pub username: String,
    #[doc(hidden)]
    pub password: String,
}

#[cfg(feature = "surrealdb")]
impl Serialize for VectorStoreAuth {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("VectorStoreAuth", 2)?;
        s.serialize_field("username", &self.username)?;
        s.serialize_field("password", "********")?;
        s.end()
    }
}

#[cfg(feature = "surrealdb")]
impl From<VectorStoreConfig> for crate::core::vector::surrealdb::SurrealDbVectorStoreConfig {
    fn from(c: VectorStoreConfig) -> Self {
        Self {
            connection_string: c.connection_string,
            namespace: c.namespace,
            database: c.database,
            table: c.table,
            dimensions: c.dimensions,
            hnsw_m: c.hnsw_m,
            hnsw_ef: c.hnsw_ef,
            auth: c.auth.map(|a| {
                crate::core::vector::surrealdb::SurrealDbAuth {
                    username: a.username,
                    password: a.password,
                }
            }),
        }
    }
}

// ─── RAG config ──────────────────────────────────────────────────────────────

/// Default search strategy for the RAG pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchType {
    Keyword,
    Semantic,
    Hybrid,
}

/// RAG pipeline configuration.
///
/// Flattens the `SearchType` enum to two scalar fields (`default_search_type`
/// and `semantic_ratio`) for TOML ergonomics.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RagConfig {
    /// Number of documents to retrieve before reranking. Default: 20
    pub retrieval_limit: usize,

    /// Number of documents to keep after reranking. Default: 5
    pub rerank_limit: usize,

    /// Maximum context length in characters for the generator. Default: 8000
    pub max_context_chars: usize,

    /// Whether to include source snippets in the response. Default: true
    pub include_snippets: bool,

    /// Default search strategy. Default: `Hybrid`
    pub default_search_type: SearchType,

    /// Semantic weight for hybrid search `[0.0, 1.0]`.
    /// Only used when `default_search_type = "hybrid"`.
    /// Default: 0.5
    pub semantic_ratio: f32,
}

impl Default for RagConfig {
    fn default() -> Self {
        Self {
            retrieval_limit: 20,
            rerank_limit: 5,
            max_context_chars: 8000,
            include_snippets: true,
            default_search_type: SearchType::Hybrid,
            semantic_ratio: 0.5,
        }
    }
}

impl TryFrom<RagConfig> for crate::core::rag::PipelineConfig {
    type Error = ConfigError;

    fn try_from(c: RagConfig) -> std::result::Result<Self, ConfigError> {
        let search_type = match c.default_search_type {
            SearchType::Keyword => crate::core::rag::SearchType::Keyword,
            SearchType::Semantic => crate::core::rag::SearchType::Semantic,
            SearchType::Hybrid => crate::core::rag::SearchType::Hybrid {
                semantic_ratio: c.semantic_ratio,
            },
        };

        Ok(Self {
            retrieval_limit: c.retrieval_limit,
            rerank_limit: c.rerank_limit,
            default_search_type: search_type,
            max_context_chars: c.max_context_chars,
            system_prompt: None,
            include_snippets: c.include_snippets,
        })
    }
}

// ─── Experimental features ───────────────────────────────────────────────────

/// Experimental feature flags.
///
/// All default to `false`. These correspond to the experimental features
/// in the core Meilisearch engine.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ExperimentalConfig {
    /// Enable Prometheus metrics endpoint.
    pub metrics: bool,
    /// Enable the logs route for real-time log streaming.
    pub logs_route: bool,
    /// Enable editing documents by function (JavaScript runtime).
    pub edit_documents_by_function: bool,
    /// Enable the `CONTAINS` filter operator.
    pub contains_filter: bool,
    /// Enable composite (multi-source) embedders.
    pub composite_embedders: bool,
    /// Enable multimodal embeddings.
    pub multimodal: bool,
    /// Enable the vector store settings in index configuration.
    pub vector_store_setting: bool,
}

impl From<ExperimentalConfig> for crate::core::ExperimentalFeatures {
    fn from(c: ExperimentalConfig) -> Self {
        Self {
            metrics: c.metrics,
            logs_route: c.logs_route,
            edit_documents_by_function: c.edit_documents_by_function,
            contains_filter: c.contains_filter,
            composite_embedders: c.composite_embedders,
            multimodal: c.multimodal,
            vector_store_setting: c.vector_store_setting,
        }
    }
}

// ─── Search defaults ─────────────────────────────────────────────────────────

/// Default parameters applied to search queries when not explicitly specified.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchDefaultsConfig {
    /// Default result limit. Default: 20
    pub limit: usize,

    /// HTML tag inserted before highlighted terms. Default: `"<em>"`
    pub highlight_pre_tag: String,

    /// HTML tag inserted after highlighted terms. Default: `"</em>"`
    pub highlight_post_tag: String,

    /// Default crop length in words. Default: 10
    pub crop_length: usize,

    /// Marker inserted at crop boundaries. Default: `"..."`
    pub crop_marker: String,
}

impl Default for SearchDefaultsConfig {
    fn default() -> Self {
        Self {
            limit: 20,
            highlight_pre_tag: "<em>".to_string(),
            highlight_post_tag: "</em>".to_string(),
            crop_length: 10,
            crop_marker: "...".to_string(),
        }
    }
}

// ─── Loading methods ─────────────────────────────────────────────────────────

impl WilysearchConfig {
    /// Build the standard figment: defaults -> `wilysearch.toml` -> env vars.
    ///
    /// Consumers can extend this with additional sources before extracting.
    pub fn figment() -> Figment {
        Figment::from(Serialized::defaults(WilysearchConfig::default()))
            .merge(Toml::file("wilysearch.toml"))
            .merge(Env::prefixed("WILYSEARCH__").split("__"))
    }

    /// Load from the standard figment (defaults + `wilysearch.toml` + env vars).
    ///
    /// Equivalent to `WilysearchConfig::from_figment(WilysearchConfig::figment())`.
    pub fn load() -> std::result::Result<Self, ConfigError> {
        Self::from_figment(Self::figment())
    }

    /// Load from a specific TOML file path, with env var overrides.
    pub fn from_file(path: impl AsRef<Path>) -> std::result::Result<Self, ConfigError> {
        let figment = Figment::from(Serialized::defaults(WilysearchConfig::default()))
            .merge(Toml::file(path.as_ref()))
            .merge(Env::prefixed("WILYSEARCH__").split("__"));
        Self::from_figment(figment)
    }

    /// Extract from a consumer-provided figment, then validate.
    pub fn from_figment(figment: Figment) -> std::result::Result<Self, ConfigError> {
        let config: Self = figment.extract()?;
        config.validate()?;
        Ok(config)
    }

    /// Validate all configuration values.
    ///
    /// Returns `Ok(())` if all values are within their valid ranges.
    /// Returns `Err(ConfigError::Validation { .. })` identifying the field,
    /// invalid value, and constraint.
    pub fn validate(&self) -> std::result::Result<(), ConfigError> {
        if self.engine.max_index_size == 0 {
            return Err(ConfigError::Validation {
                field: "engine.max_index_size".to_string(),
                value: "0".to_string(),
                message: "must be greater than 0".to_string(),
            });
        }

        if self.engine.max_task_db_size == 0 {
            return Err(ConfigError::Validation {
                field: "engine.max_task_db_size".to_string(),
                value: "0".to_string(),
                message: "must be greater than 0".to_string(),
            });
        }

        if self.preprocessing.typo.max_edit_distance > 3 {
            return Err(ConfigError::Validation {
                field: "preprocessing.typo.max_edit_distance".to_string(),
                value: self.preprocessing.typo.max_edit_distance.to_string(),
                message: "must be between 0 and 3".to_string(),
            });
        }

        #[cfg(feature = "surrealdb")]
        if let Some(ref vs) = self.vector_store {
            if vs.dimensions == 0 {
                return Err(ConfigError::Validation {
                    field: "vector_store.dimensions".to_string(),
                    value: "0".to_string(),
                    message: "must be greater than 0".to_string(),
                });
            }
        }

        if self.rag.retrieval_limit == 0 {
            return Err(ConfigError::Validation {
                field: "rag.retrieval_limit".to_string(),
                value: "0".to_string(),
                message: "must be greater than 0".to_string(),
            });
        }

        if self.rag.rerank_limit == 0 {
            return Err(ConfigError::Validation {
                field: "rag.rerank_limit".to_string(),
                value: "0".to_string(),
                message: "must be greater than 0".to_string(),
            });
        }

        if self.rag.max_context_chars == 0 {
            return Err(ConfigError::Validation {
                field: "rag.max_context_chars".to_string(),
                value: "0".to_string(),
                message: "must be greater than 0".to_string(),
            });
        }

        if self.search_defaults.limit == 0 {
            return Err(ConfigError::Validation {
                field: "search_defaults.limit".to_string(),
                value: "0".to_string(),
                message: "must be greater than 0".to_string(),
            });
        }

        if !(0.0..=1.0).contains(&self.rag.semantic_ratio) {
            return Err(ConfigError::Validation {
                field: "rag.semantic_ratio".to_string(),
                value: self.rag.semantic_ratio.to_string(),
                message: "must be between 0.0 and 1.0".to_string(),
            });
        }

        // default_search_type is now an enum -- invalid values are rejected at deserialization

        Ok(())
    }
}

/// Implement `Provider` so consumers can use a `WilysearchConfig` as a figment
/// source in their own config layering.
impl Provider for WilysearchConfig {
    fn metadata(&self) -> figment::Metadata {
        figment::Metadata::named("WilysearchConfig")
    }

    fn data(
        &self,
    ) -> std::result::Result<
        figment::value::Map<figment::Profile, figment::value::Dict>,
        figment::Error,
    > {
        Serialized::defaults(self).data()
    }
}

// ─── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_is_valid() {
        let config = WilysearchConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_toml_deserialization_full() {
        let toml = r#"
            [engine]
            db_path = "/var/lib/wilysearch"
            max_index_size = 53687091200
            max_task_db_size = 5368709120

            [preprocessing.typo]
            enabled = true
            maxEditDistance = 2

            [preprocessing.synonyms]
            enabled = true

            [preprocessing.normalization]
            lowercase = true
            trim = true

            [rag]
            retrieval_limit = 30
            rerank_limit = 10
            max_context_chars = 16000
            include_snippets = false
            default_search_type = "keyword"
            semantic_ratio = 0.0

            [experimental]
            metrics = true
            contains_filter = true

            [search_defaults]
            limit = 50
            highlight_pre_tag = "<mark>"
            highlight_post_tag = "</mark>"
            crop_length = 20
            crop_marker = "…"
        "#;

        let figment = Figment::from(Serialized::defaults(WilysearchConfig::default()))
            .merge(Toml::string(toml));
        let config: WilysearchConfig = figment.extract().unwrap();

        assert_eq!(config.engine.db_path, PathBuf::from("/var/lib/wilysearch"));
        assert_eq!(config.engine.max_index_size, 53687091200);
        assert_eq!(config.rag.retrieval_limit, 30);
        assert_eq!(config.rag.default_search_type, SearchType::Keyword);
        assert!(!config.rag.include_snippets);
        assert!(config.experimental.metrics);
        assert!(config.experimental.contains_filter);
        assert_eq!(config.search_defaults.limit, 50);
        assert_eq!(config.search_defaults.highlight_pre_tag, "<mark>");
        assert_eq!(config.search_defaults.crop_marker, "…");
    }

    #[test]
    fn test_toml_deserialization_partial() {
        let toml = r#"
            [engine]
            db_path = "/custom/path"
        "#;

        let figment = Figment::from(Serialized::defaults(WilysearchConfig::default()))
            .merge(Toml::string(toml));
        let config: WilysearchConfig = figment.extract().unwrap();

        assert_eq!(config.engine.db_path, PathBuf::from("/custom/path"));
        // All other fields should be defaults
        assert_eq!(config.engine.max_index_size, 100 * 1024 * 1024 * 1024);
        assert_eq!(config.rag.retrieval_limit, 20);
        assert_eq!(config.search_defaults.limit, 20);
    }

    #[test]
    fn test_toml_unknown_keys_ignored() {
        let toml = r#"
            [engine]
            db_path = "data.ms"
            some_future_field = "hello"

            [unknown_section]
            foo = "bar"
        "#;

        let figment = Figment::from(Serialized::defaults(WilysearchConfig::default()))
            .merge(Toml::string(toml));
        let config: WilysearchConfig = figment.extract().unwrap();
        assert_eq!(config.engine.db_path, PathBuf::from("data.ms"));
    }

    #[test]
    fn test_validation_catches_zero_index_size() {
        let mut config = WilysearchConfig::default();
        config.engine.max_index_size = 0;
        let err = config.validate().unwrap_err();
        match err {
            ConfigError::Validation { field, .. } => {
                assert_eq!(field, "engine.max_index_size");
            }
            _ => panic!("expected Validation error"),
        }
    }

    #[test]
    fn test_validation_catches_zero_task_db_size() {
        let mut config = WilysearchConfig::default();
        config.engine.max_task_db_size = 0;
        let err = config.validate().unwrap_err();
        match err {
            ConfigError::Validation { field, .. } => {
                assert_eq!(field, "engine.max_task_db_size");
            }
            _ => panic!("expected Validation error"),
        }
    }

    #[test]
    fn test_validation_catches_invalid_edit_distance() {
        let mut config = WilysearchConfig::default();
        config.preprocessing.typo.max_edit_distance = 5;
        let err = config.validate().unwrap_err();
        match err {
            ConfigError::Validation { field, .. } => {
                assert_eq!(field, "preprocessing.typo.max_edit_distance");
            }
            _ => panic!("expected Validation error"),
        }
    }

    #[test]
    fn test_validation_catches_invalid_semantic_ratio() {
        let mut config = WilysearchConfig::default();
        config.rag.semantic_ratio = 1.5;
        let err = config.validate().unwrap_err();
        match err {
            ConfigError::Validation { field, .. } => {
                assert_eq!(field, "rag.semantic_ratio");
            }
            _ => panic!("expected Validation error"),
        }
    }

    #[test]
    fn test_invalid_search_type_rejected_at_deserialization() {
        let toml = r#"
            [engine]
            db_path = "/tmp/test"

            [rag]
            default_search_type = "vector"
        "#;
        let figment = Figment::from(Serialized::defaults(WilysearchConfig::default()))
            .merge(Toml::string(toml));
        let result: std::result::Result<WilysearchConfig, _> = figment.extract();
        assert!(result.is_err(), "invalid search type should fail at deserialization");
    }

    #[test]
    fn test_engine_config_to_meilisearch_options() {
        let ec = EngineConfig {
            db_path: "/tmp/test".into(),
            max_index_size: 42,
            max_task_db_size: 7,
        };
        let opts: MeilisearchOptions = ec.into();
        assert_eq!(opts.db_path, PathBuf::from("/tmp/test"));
        assert_eq!(opts.max_index_size, 42);
        assert_eq!(opts.max_task_db_size, 7);
    }

    #[test]
    fn test_rag_config_to_pipeline_config_hybrid() {
        let rc = RagConfig {
            default_search_type: SearchType::Hybrid,
            semantic_ratio: 0.7,
            ..Default::default()
        };
        let pc: crate::core::rag::PipelineConfig = rc.try_into().unwrap();
        assert_eq!(pc.retrieval_limit, 20);
        match pc.default_search_type {
            crate::core::rag::SearchType::Hybrid { semantic_ratio } => {
                assert!((semantic_ratio - 0.7).abs() < f32::EPSILON);
            }
            _ => panic!("expected Hybrid"),
        }
    }

    #[test]
    fn test_rag_config_to_pipeline_config_keyword() {
        let rc = RagConfig {
            default_search_type: SearchType::Keyword,
            ..Default::default()
        };
        let pc: crate::core::rag::PipelineConfig = rc.try_into().unwrap();
        assert!(matches!(
            pc.default_search_type,
            crate::core::rag::SearchType::Keyword
        ));
    }

    #[test]
    fn test_rag_config_to_pipeline_config_semantic() {
        let rc = RagConfig {
            default_search_type: SearchType::Semantic,
            ..Default::default()
        };
        let pc: crate::core::rag::PipelineConfig = rc.try_into().unwrap();
        assert!(matches!(
            pc.default_search_type,
            crate::core::rag::SearchType::Semantic
        ));
    }

    #[test]
    fn test_env_var_override() {
        // Use figment::Jail for hermetic env testing
        figment::Jail::expect_with(|jail| {
            jail.set_env("WILYSEARCH__ENGINE__DB_PATH", "/env/path");
            jail.set_env("WILYSEARCH__RAG__RETRIEVAL_LIMIT", "42");
            jail.set_env("WILYSEARCH__SEARCH_DEFAULTS__LIMIT", "100");

            let config: WilysearchConfig = WilysearchConfig::figment().extract()?;
            assert_eq!(config.engine.db_path, PathBuf::from("/env/path"));
            assert_eq!(config.rag.retrieval_limit, 42);
            assert_eq!(config.search_defaults.limit, 100);
            Ok(())
        });
    }

    #[test]
    fn test_env_overrides_file() {
        figment::Jail::expect_with(|jail| {
            jail.create_file(
                "wilysearch.toml",
                r#"
                [engine]
                db_path = "/from/file"

                [rag]
                retrieval_limit = 10
                "#,
            )?;

            // Env should override the file value
            jail.set_env("WILYSEARCH__ENGINE__DB_PATH", "/from/env");

            let config: WilysearchConfig = WilysearchConfig::figment().extract()?;
            assert_eq!(config.engine.db_path, PathBuf::from("/from/env"));
            // File value not overridden by env
            assert_eq!(config.rag.retrieval_limit, 10);
            Ok(())
        });
    }

    #[test]
    fn test_experimental_config_to_core() {
        let ec = ExperimentalConfig {
            metrics: true,
            contains_filter: true,
            ..Default::default()
        };
        let core: crate::core::ExperimentalFeatures = ec.into();
        assert!(core.metrics);
        assert!(core.contains_filter);
        assert!(!core.logs_route);
    }

    #[test]
    fn test_empty_toml_uses_defaults() {
        figment::Jail::expect_with(|jail| {
            jail.create_file("wilysearch.toml", "")?;
            let config: WilysearchConfig = WilysearchConfig::figment().extract()?;
            let default = WilysearchConfig::default();
            assert_eq!(config.engine.db_path, default.engine.db_path);
            assert_eq!(config.rag.retrieval_limit, default.rag.retrieval_limit);
            assert_eq!(config.search_defaults.limit, default.search_defaults.limit);
            Ok(())
        });
    }

    #[test]
    fn test_provider_impl() {
        let config = WilysearchConfig {
            engine: EngineConfig {
                db_path: "/custom".into(),
                ..Default::default()
            },
            ..Default::default()
        };

        // Use config as a figment provider, then extract
        let figment = Figment::from(&config);
        let extracted: WilysearchConfig = figment.extract().unwrap();
        assert_eq!(extracted.engine.db_path, PathBuf::from("/custom"));
    }
}
