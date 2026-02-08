//! Integration tests for the unified configuration system (`src/config.rs`).
//!
//! These tests verify that configuration structs load, validate, convert to
//! core types, and produce working engines via the `Engine::with_config` and
//! `Engine::from_config_file` constructors.

mod common;

use figment::providers::Format;
use figment::Figment;
use tempfile::TempDir;
use wilysearch::config::*;
use wilysearch::engine::Engine;
use wilysearch::traits::*;
use wilysearch::types::*;

// ─── Test 1: Engine with default config ─────────────────────────────────────

/// Create an engine from `WilysearchConfig::default()` (with a temp dir for
/// db_path). Verify it can create an index, add documents, and search.
#[test]
fn test_engine_with_default_config() {
    let temp = TempDir::new().unwrap();
    let mut config = WilysearchConfig::default();
    config.engine.db_path = temp.path().to_path_buf();
    config.engine.max_index_size = 100 * 1024 * 1024;
    config.engine.max_task_db_size = 10 * 1024 * 1024;

    config.validate().expect("default config should be valid");

    let engine = Engine::with_config(config).expect("failed to create engine");

    engine
        .create_index(&CreateIndexRequest {
            uid: "test".to_string(),
            primary_key: Some("id".to_string()),
        })
        .expect("failed to create index");

    let docs = vec![serde_json::json!({"id": 1, "title": "hello world"})];
    engine
        .add_or_replace_documents("test", &docs, &AddDocumentsQuery::default())
        .expect("failed to add docs");

    let results = engine
        .search(
            "test",
            &SearchRequest {
                q: Some("hello".to_string()),
                ..Default::default()
            },
        )
        .expect("search failed");
    assert!(!results.hits.is_empty(), "search should return at least one hit");
}

// ─── Test 2: Engine from config file ────────────────────────────────────────

/// Write a TOML config to a temp file with custom engine sizes, load it via
/// `Engine::from_config_file`, and verify the engine works end-to-end.
#[test]
fn test_engine_from_config_file() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("db");
    let config_path = temp.path().join("wilysearch.toml");

    let toml_content = format!(
        r#"
[engine]
db_path = "{}"
max_index_size = 104857600
max_task_db_size = 10485760

[search_defaults]
limit = 5
crop_length = 15

[rag]
retrieval_limit = 10
default_search_type = "keyword"
"#,
        db_path.display()
    );

    std::fs::write(&config_path, &toml_content).expect("failed to write config file");

    let engine =
        Engine::from_config_file(&config_path).expect("failed to create engine from file config");

    engine
        .create_index(&CreateIndexRequest {
            uid: "movies".to_string(),
            primary_key: Some("id".to_string()),
        })
        .expect("failed to create index");

    let docs = common::sample_movies();
    engine
        .add_or_replace_documents("movies", &docs, &AddDocumentsQuery::default())
        .expect("failed to add docs");

    let results = engine
        .search(
            "movies",
            &SearchRequest {
                q: Some("dark knight".to_string()),
                ..Default::default()
            },
        )
        .expect("search failed");
    assert!(!results.hits.is_empty(), "should find 'The Dark Knight'");
}

// ─── Test 3: Empty TOML creates a working engine ────────────────────────────

/// An empty TOML file should produce valid defaults. We override `db_path`
/// after loading (since the default `data.ms` is relative and we need a temp
/// dir for test isolation), then verify the engine works.
#[test]
fn test_empty_toml_creates_working_engine() {
    let temp = TempDir::new().unwrap();
    let config_path = temp.path().join("empty.toml");
    std::fs::write(&config_path, "").expect("failed to write empty config");

    let mut config =
        WilysearchConfig::from_file(&config_path).expect("empty TOML should parse successfully");

    // Override db_path for test isolation
    config.engine.db_path = temp.path().join("db");
    config.engine.max_index_size = 100 * 1024 * 1024;
    config.engine.max_task_db_size = 10 * 1024 * 1024;

    // Defaults should still be valid
    config.validate().expect("default config should be valid");

    let engine = Engine::with_config(config).expect("engine from empty TOML should work");

    engine
        .create_index(&CreateIndexRequest {
            uid: "empty_test".to_string(),
            primary_key: Some("id".to_string()),
        })
        .expect("failed to create index");

    let health = engine.health().expect("health check failed");
    assert_eq!(health.status, "available");
}

// ─── Test 4: Invalid config produces clear error message ────────────────────

/// Setting `max_index_size = 0` should trigger a `ConfigError::Validation`
/// that includes the field name `engine.max_index_size`.
#[test]
fn test_invalid_config_error_message() {
    let mut config = WilysearchConfig::default();
    config.engine.max_index_size = 0;

    let err = config.validate().unwrap_err();
    let err_string = err.to_string();

    // Verify the error message contains the field name for debuggability
    assert!(
        err_string.contains("engine.max_index_size"),
        "error should name the invalid field, got: {err_string}"
    );
    assert!(
        err_string.contains("must be greater than 0"),
        "error should explain the constraint, got: {err_string}"
    );

    // Also verify the structured error variant
    match err {
        ConfigError::Validation {
            ref field,
            ref value,
            ref message,
        } => {
            assert_eq!(field, "engine.max_index_size");
            assert_eq!(value, "0");
            assert_eq!(message, "must be greater than 0");
        }
        _ => panic!("expected ConfigError::Validation, got: {err:?}"),
    }
}

/// Setting `max_task_db_size = 0` should also produce a clear validation error.
#[test]
fn test_invalid_task_db_size_error() {
    let mut config = WilysearchConfig::default();
    config.engine.max_task_db_size = 0;

    let err = config.validate().unwrap_err();
    match err {
        ConfigError::Validation { ref field, .. } => {
            assert_eq!(field, "engine.max_task_db_size");
        }
        _ => panic!("expected ConfigError::Validation, got: {err:?}"),
    }
}

/// Invalid `rag.default_search_type` should be rejected at deserialization.
#[test]
fn test_invalid_search_type_error() {
    let toml = r#"
        [engine]
        db_path = "/tmp/test"

        [rag]
        default_search_type = "fulltext"
    "#;
    let figment = Figment::from(figment::providers::Serialized::defaults(
        WilysearchConfig::default(),
    ))
    .merge(figment::providers::Toml::string(toml));
    let result: std::result::Result<WilysearchConfig, _> = figment.extract();
    assert!(
        result.is_err(),
        "invalid search type 'fulltext' should fail at deserialization"
    );
}

/// `semantic_ratio` outside `[0.0, 1.0]` should be caught by validation.
#[test]
fn test_invalid_semantic_ratio_error() {
    let mut config = WilysearchConfig::default();
    config.rag.semantic_ratio = -0.1;

    let err = config.validate().unwrap_err();
    match err {
        ConfigError::Validation { ref field, .. } => {
            assert_eq!(field, "rag.semantic_ratio");
        }
        _ => panic!("expected ConfigError::Validation, got: {err:?}"),
    }
}

// ─── Test 5: Reference TOML parses ──────────────────────────────────────────

/// Parse `wilysearch.reference.toml` from the project root and verify it
/// produces a valid configuration that matches the documented defaults.
///
/// This test will pass once `wilysearch.reference.toml` is created (task #12).
/// If the file does not exist, the test is skipped rather than failed.
#[test]
fn test_reference_toml_parses() {
    let reference_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("wilysearch.reference.toml");

    if !reference_path.exists() {
        eprintln!(
            "SKIPPED: {} does not exist yet (waiting for task #12)",
            reference_path.display()
        );
        return;
    }

    let config = WilysearchConfig::from_file(&reference_path)
        .expect("reference TOML should parse without errors");

    // The reference file should document defaults, so values should match
    let defaults = WilysearchConfig::default();
    assert_eq!(
        config.engine.max_index_size, defaults.engine.max_index_size,
        "reference TOML max_index_size should match default"
    );
    assert_eq!(
        config.engine.max_task_db_size, defaults.engine.max_task_db_size,
        "reference TOML max_task_db_size should match default"
    );
    assert_eq!(
        config.rag.retrieval_limit, defaults.rag.retrieval_limit,
        "reference TOML retrieval_limit should match default"
    );
    assert_eq!(
        config.rag.default_search_type, defaults.rag.default_search_type,
        "reference TOML default_search_type should match default"
    );
    assert!(
        (config.rag.semantic_ratio - defaults.rag.semantic_ratio).abs() < f32::EPSILON,
        "reference TOML semantic_ratio should match default"
    );
    assert_eq!(
        config.search_defaults.limit, defaults.search_defaults.limit,
        "reference TOML search limit should match default"
    );
    assert_eq!(
        config.search_defaults.highlight_pre_tag, defaults.search_defaults.highlight_pre_tag,
        "reference TOML highlight_pre_tag should match default"
    );
    assert_eq!(
        config.search_defaults.crop_length, defaults.search_defaults.crop_length,
        "reference TOML crop_length should match default"
    );
    assert_eq!(
        config.search_defaults.crop_marker, defaults.search_defaults.crop_marker,
        "reference TOML crop_marker should match default"
    );

    // Experimental flags should all be false (off by default)
    assert!(!config.experimental.metrics, "reference TOML metrics should be false");
    assert!(
        !config.experimental.logs_route,
        "reference TOML logs_route should be false"
    );
    assert!(
        !config.experimental.contains_filter,
        "reference TOML contains_filter should be false"
    );

    config.validate().expect("reference TOML should pass validation");
}

// ─── Additional config integration tests ────────────────────────────────────

/// Verify that `WilysearchConfig::from_file` rejects a TOML file with an
/// invalid value that figment can parse but validation catches.
#[test]
fn test_from_file_validates_on_load() {
    let temp = TempDir::new().unwrap();
    let config_path = temp.path().join("bad.toml");

    let toml_content = r#"
[engine]
max_index_size = 0
"#;
    std::fs::write(&config_path, toml_content).expect("failed to write config");

    let result = WilysearchConfig::from_file(&config_path);
    assert!(result.is_err(), "from_file should fail on invalid config");

    let err = result.unwrap_err();
    let err_string = err.to_string();
    assert!(
        err_string.contains("engine.max_index_size"),
        "error should identify the bad field, got: {err_string}"
    );
}

/// Verify partial TOML files merge correctly with defaults: only the
/// specified fields change, everything else stays at defaults.
#[test]
fn test_partial_toml_preserves_defaults() {
    let temp = TempDir::new().unwrap();
    let config_path = temp.path().join("partial.toml");

    let toml_content = r#"
[search_defaults]
limit = 42
"#;
    std::fs::write(&config_path, toml_content).expect("failed to write config");

    let config = WilysearchConfig::from_file(&config_path).expect("partial TOML should parse");
    let defaults = WilysearchConfig::default();

    // The overridden field should differ
    assert_eq!(config.search_defaults.limit, 42);

    // Everything else should be default
    assert_eq!(config.engine.max_index_size, defaults.engine.max_index_size);
    assert_eq!(
        config.engine.max_task_db_size,
        defaults.engine.max_task_db_size
    );
    assert_eq!(config.rag.retrieval_limit, defaults.rag.retrieval_limit);
    assert_eq!(
        config.search_defaults.highlight_pre_tag,
        defaults.search_defaults.highlight_pre_tag
    );
    assert_eq!(
        config.search_defaults.crop_length,
        defaults.search_defaults.crop_length
    );
}

/// Verify that `EngineConfig` converts correctly to `MeilisearchOptions`.
#[test]
fn test_engine_config_converts_to_meilisearch_options() {
    use wilysearch::core::MeilisearchOptions;

    let temp = TempDir::new().unwrap();
    let ec = EngineConfig {
        db_path: temp.path().to_path_buf(),
        max_index_size: 200 * 1024 * 1024,
        max_task_db_size: 20 * 1024 * 1024,
    };

    let opts: MeilisearchOptions = ec.into();
    assert_eq!(opts.db_path, temp.path().to_path_buf());
    assert_eq!(opts.max_index_size, 200 * 1024 * 1024);
    assert_eq!(opts.max_task_db_size, 20 * 1024 * 1024);
}

/// Verify that a config built programmatically can be used as a figment
/// Provider (round-trip test).
#[test]
fn test_config_provider_round_trip() {
    use figment::Figment;

    let original = WilysearchConfig {
        engine: EngineConfig {
            db_path: "/round/trip/test".into(),
            max_index_size: 555,
            max_task_db_size: 111,
        },
        rag: RagConfig {
            retrieval_limit: 77,
            default_search_type: SearchType::Semantic,
            ..Default::default()
        },
        search_defaults: SearchDefaultsConfig {
            limit: 99,
            ..Default::default()
        },
        ..Default::default()
    };

    // Use the config as a figment provider and extract back
    let figment = Figment::from(&original);
    let extracted: WilysearchConfig = figment.extract().unwrap();

    assert_eq!(
        extracted.engine.db_path,
        std::path::PathBuf::from("/round/trip/test")
    );
    assert_eq!(extracted.engine.max_index_size, 555);
    assert_eq!(extracted.engine.max_task_db_size, 111);
    assert_eq!(extracted.rag.retrieval_limit, 77);
    assert_eq!(extracted.rag.default_search_type, SearchType::Semantic);
    assert_eq!(extracted.search_defaults.limit, 99);
}

/// Verify that TOML with all experimental flags enabled parses correctly.
#[test]
fn test_experimental_flags_from_toml() {
    let temp = TempDir::new().unwrap();
    let config_path = temp.path().join("experimental.toml");

    let toml_content = r#"
[experimental]
metrics = true
logs_route = true
edit_documents_by_function = true
contains_filter = true
composite_embedders = true
multimodal = true
vector_store_setting = true
"#;
    std::fs::write(&config_path, toml_content).expect("failed to write config");

    let config =
        WilysearchConfig::from_file(&config_path).expect("experimental TOML should parse");

    assert!(config.experimental.metrics);
    assert!(config.experimental.logs_route);
    assert!(config.experimental.edit_documents_by_function);
    assert!(config.experimental.contains_filter);
    assert!(config.experimental.composite_embedders);
    assert!(config.experimental.multimodal);
    assert!(config.experimental.vector_store_setting);
}

/// Verify that `ExperimentalConfig` converts to `core::ExperimentalFeatures`.
#[test]
fn test_experimental_config_converts_to_core() {
    let ec = ExperimentalConfig {
        metrics: true,
        contains_filter: true,
        vector_store_setting: true,
        ..Default::default()
    };

    let core: wilysearch::core::ExperimentalFeatures = ec.into();
    assert!(core.metrics);
    assert!(core.contains_filter);
    assert!(core.vector_store_setting);
    assert!(!core.logs_route);
    assert!(!core.multimodal);
}

/// Verify that `RagConfig` with `"hybrid"` converts to the correct
/// `PipelineConfig` with the right semantic ratio.
#[test]
fn test_rag_config_hybrid_conversion() {
    let rc = RagConfig {
        retrieval_limit: 30,
        rerank_limit: 8,
        max_context_chars: 4000,
        include_snippets: false,
        default_search_type: SearchType::Hybrid,
        semantic_ratio: 0.3,
    };

    let pc: wilysearch::core::rag::PipelineConfig = rc.try_into().unwrap();
    assert_eq!(pc.retrieval_limit, 30);
    assert_eq!(pc.rerank_limit, 8);
    assert_eq!(pc.max_context_chars, 4000);
    assert!(!pc.include_snippets);
    match pc.default_search_type {
        wilysearch::core::rag::SearchType::Hybrid { semantic_ratio } => {
            assert!((semantic_ratio - 0.3).abs() < f32::EPSILON);
        }
        other => panic!("expected Hybrid, got: {other:?}"),
    }
}

/// Verify that invalid search type is rejected at deserialization, not TryFrom.
#[test]
fn test_rag_config_invalid_search_type_deserialization() {
    // With a typed enum, invalid values are caught by serde, so we test TOML parsing
    let toml = r#"
        [engine]
        db_path = "/tmp/test"

        [rag]
        default_search_type = "bm25"
    "#;
    let figment = Figment::from(figment::providers::Serialized::defaults(
        WilysearchConfig::default(),
    ))
    .merge(figment::providers::Toml::string(toml));
    let result: std::result::Result<WilysearchConfig, _> = figment.extract();
    assert!(
        result.is_err(),
        "invalid search type 'bm25' should fail at deserialization"
    );
}
